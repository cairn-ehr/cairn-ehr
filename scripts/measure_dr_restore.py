#!/usr/bin/env python3
"""The #512 paper-parity measurement rig for the DR restore ceremony.

House rule 7 owes this ceremony a measured §1.2 figure. DR slice 1's plan states
the budget and calls it unmeasured:

    a restore completes within 10 minutes unattended after the operator's last
    keystroke; the operator needs one secret and no knowledge of the dead node's
    configuration

This rig produces the time half. The cognitive-load half — *no knowledge of the
dead node's config* — is pinned separately and mechanically, by
`crates/cairn-node/tests/restore_needs_nothing_about_the_dead_node.rs`.

**The measured leg is the shipped CLI, deliberately.** The leg graded against the
budget is `cairn-node restore` as an operator would run it. (Seed and backup are
also timed, but only reported: seed is context and backup belongs to #552.)
Timing a library call instead
would flatter the architecture by dropping process start, schema load, the
prompt and the summary — and the budget is a promise about a ceremony, not about
a function.

**Seeding is NOT measured**, and is not meant to be. It exists only to produce a
medium at a realistic scale, and it runs through the `seed_measurement_corpus`
example rather than the CLI because a hundred thousand process starts would cost
six hours of overhead measuring nothing.

# The pseudo-terminal, and why it is here

A restore prompts for the OLD node's recovery code through `rpassword`, which
reads `/dev/tty` and **fails on any non-tty** — there is no flag and no
environment variable for it, unlike the new key's passphrase. So a restore **of a
sealed node's medium** — the only kind the budget describes, since an
`--insecure-plaintext` node writes no sealed export and never reaches the prompt
— cannot be scripted, piped or run from cron, and this rig has to allocate a pty
to drive one. That is a finding about the product, not a convenience of the rig;
see the issue filed from the measurement run.

# Usage

    python3 scripts/measure_dr_restore.py --sizes 100,1000,2500,5000

Sizes are **patient counts**; each patient yields 3 events (one
`identity.registration.asserted` plus two `demographic.field.asserted` — name and
date of birth) plus `--meds-per-patient` born-sealed clinical events. The run
adds 3 more for its one-off warm-up registration, which is why the published
points are 2 003 / 20 003 / 50 003 / 100 003 rather than round numbers. Run
`--self-test` to exercise the parsers without a database.

Requires a PostgreSQL cluster with `cairn_pgx` (see `scripts/pg-target.sh`), and
release builds of `cairn-node` and the `seed_measurement_corpus` example.
"""

from __future__ import annotations

import argparse
import errno
import getpass
import os
import pty
import re
import secrets
import select
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

#: #512's budget for a whole restore, in seconds. **Not a parameter.** Making it
#: one is how a measurement quietly gets graded against a budget somebody widened
#: to make a run pass; house rule 7 says a figure outside its budget is the
#: finding, so the budget has to be the fixed thing.
BUDGET_SECONDS = 600

#: How long to wait for the recovery-code prompt before giving up. Generous: the
#: prompt comes after the node plane is applied, which grows with the medium.
PROMPT_TIMEOUT_SECONDS = 900

#: Ceiling on any single child process, so a wedged run fails the rig instead of
#: hanging a session. Comfortably above the budget it is measuring. Named for the
#: restore because that is the leg it exists to bound, but `run()` applies it to
#: `init`, the warm-up registration and `backup` too — the seed has its own,
#: because it is the one leg that legitimately grows past a restore.
SUBPROCESS_TIMEOUT_SECONDS = 3600

#: Seeding 100k events through the production orchestrators is slower than
#: restoring them and is not the thing being graded, so it gets its own headroom.
SEED_TIMEOUT_SECONDS = 7200

#: Ceiling on one `psql` statement. Short on purpose: these are `CREATE`/`DROP`
#: DATABASE and a `CREATE EXTENSION`, so anything slow here is a lock held by a
#: leaked backend, which is a rig failure and not work in progress.
PSQL_TIMEOUT_SECONDS = 120

#: The curve actually run and published, so a bare invocation reproduces the
#: recorded figures rather than points that do not line up with them. Pinned
#: against `crates/cairn-node/results/RUNBOOK.md` by a test, because this drifted
#: once already and was caught by eye in a follow-up commit, not by the suite.
DEFAULT_SIZES = "100,1000,2500,5000"

#: Meds per patient in the published run. With the 3 events a registration
#: authors, this makes each patient 20 events.
DEFAULT_MEDS_PER_PATIENT = 17

#: `cairn-node restore` re-prompts this many times for the old recovery code
#: (`RECOVERY_CODE_ATTEMPTS` in `crates/cairn-node/src/main.rs`). The rig must be
#: willing to answer every one of them — see `restore_under_pty`.
RECOVERY_CODE_ATTEMPTS = 3


class RigError(RuntimeError):
    """The rig could not produce an honest number, and says why rather than guessing."""


# ---------------------------------------------------------------------------
# Pure parsing — the half that decides what number gets written down
# ---------------------------------------------------------------------------

#: `generate_recovery_code` (crates/cairn-keystore/src/seal.rs) emits 160 bits of
#: Crockford base32 — 32 characters — chunked in fives and dash-joined, so the
#: shipped shape is exactly six groups of five plus a final group of two, 38
#: characters in all.
#:
#: The pattern is deliberately looser than that (it admits 6 to 8 groups, each
#: trailing one 2-5 characters) so a future code built from a different number of
#: bytes still parses rather than silently failing to match. It is anchored on the
#: code's SHAPE rather than on the banner around it, so a reworded banner does not
#: stop the rig finding the code. Nothing else `init` prints can match: the
#: fingerprint's groups are four characters wide and the node id is lower-case hex.
_RECOVERY_CODE = re.compile(r"\b([A-Z0-9]{5}(?:-[A-Z0-9]{2,5}){5,7})\b")

_CLINICAL_SUMMARY = re.compile(
    r"clinical records:\s*(\d+)\s+applied,\s*(\d+)\s+already present,\s*"
    r"(\d+)\s+refused\s*\(of\s*(\d+)\s+on the medium\)"
)


def parse_recovery_code(text: str) -> str:
    """Pull the shown-once recovery code out of `cairn-node init` output.

    Raises rather than returning `None` on a miss. Silence here would strand the
    rig at an unanswerable prompt minutes later, which surfaces as a timeout
    during the restore and reads exactly like a slow restore — the one thing this
    rig exists to measure. Failing at the point the code should have appeared
    names the real cause.
    """
    # A plain first-match search. There was once a `"RECOVERY" not in candidate`
    # filter here to exclude the banner; it was dead code, because "RECOVERY" is
    # eight consecutive alphanumerics and no group in the pattern is wider than
    # five, so no candidate can ever contain it. The banner is excluded by the
    # SHAPE of the pattern, which is where that reasoning belongs.
    match = _RECOVERY_CODE.search(text)
    if match:
        return match.group(1)
    raise RigError(
        "no recovery code found in `init` output — the rig cannot answer the "
        "restore's prompt without it. Output was:\n" + text
    )


@dataclass(frozen=True)
class ClinicalSummary:
    """The restore's own count of what it recovered, as printed to the operator."""

    applied: int
    already_present: int
    refused: int
    on_medium: int

    @property
    def is_complete(self) -> bool:
        """Did every clinical record on the medium land?

        `already_present` counts toward completeness on purpose: it is the door
        reporting an **idempotent no-op**, not a loss, so a resumed restore into a
        partially-populated database is complete. Scoring it as a shortfall would
        make a resumed restore look like a failed one.

        `on_medium > 0` is the guard against the opposite error. An entirely empty
        medium satisfies every other clause vacuously — nothing refused, nothing
        missing — and would be recorded as a complete restore taking about a
        second, which is #500's signature and the exact fast wrong number this rig
        exists to refuse. Today `restore` happens not to print the summary line at
        all when it carried no clinical records, so the case is unreachable through
        the CLI; that is an implementation detail of a Rust file this rig cannot
        see change, so the invariant is stated HERE rather than depended on there.
        """
        return (
            self.on_medium > 0
            and self.refused == 0
            and (self.applied + self.already_present) >= self.on_medium
        )


def parse_clinical_summary(text: str) -> ClinicalSummary:
    """Read the restore's clinical summary line.

    The rig refuses to record a timing for an incomplete restore, and this is
    where that judgment gets its inputs. A restore that applied nothing is
    **fast**, and #500's own signature is a clean-looking summary over an empty
    restore — so a fast wrong number is precisely what would end up quoted
    against the budget.
    """
    match = _CLINICAL_SUMMARY.search(text)
    if not match:
        raise RigError(
            "no clinical summary line in the restore output — cannot tell what "
            "was recovered, so cannot record a timing. Output was:\n" + text
        )
    applied, already, refused, on_medium = (int(g) for g in match.groups())
    return ClinicalSummary(applied, already, refused, on_medium)


@dataclass(frozen=True)
class Measurement:
    """One point on the scaling curve.

    Both event counts here — `on_medium` and `applied` — come from the restore's
    own summary line, so neither can drift from what was measured. An independent
    `events` field derived from the SEED parameters was removed after a test caught
    it disagreeing with them: a count spelled a second way is a second thing that
    can be wrong, and that one would have been wrong in the results file.
    """

    seed_s: float
    backup_s: float
    restore_s: float
    applied: int
    on_medium: int

    @property
    def within_budget(self) -> bool:
        return self.restore_s <= BUDGET_SECONDS


def format_results_table(rows: list[Measurement]) -> str:
    """Render the curve as a markdown table, verdict beside each number.

    The verdict goes **in the table**, not in prose below it: house rule 7 makes
    the budget comparison the point of the measurement, and a reader should not
    have to reconcile a number here against a claim three paragraphs down.
    """
    # The budget column is LABELLED from `BUDGET_SECONDS` rather than spelled out,
    # so the header and the verdict can never disagree about what was tested.
    out = [
        (
            f"| Events on medium | Seed (s) | Backup (s) | **Restore (s)** | Applied | "
            f"≤ {BUDGET_SECONDS // 60} min |"
        ),
        "|---:|---:|---:|---:|---:|:--|",
    ]
    for r in rows:
        verdict = "PASS" if r.within_budget else "**FAIL**"
        out.append(
            f"| {r.on_medium} | {r.seed_s:.1f} | {r.backup_s:.1f} | "
            f"**{r.restore_s:.1f}** | {r.applied} | {verdict} |"
        )
    return "\n".join(out)


# ---------------------------------------------------------------------------
# Driving the real binaries
# ---------------------------------------------------------------------------


def run(
    cmd: list[str],
    env: dict[str, str],
    what: str,
    timeout: int = SUBPROCESS_TIMEOUT_SECONDS,
    stream: bool = False,
) -> str:
    """Run a command to completion, returning combined output, raising on failure.

    `stream=True` lets a long child's output reach the operator live instead of
    being buffered until it exits. The seed is the reason: it is the slowest leg,
    it can run for minutes, and captured output means an operator watching an
    overnight run cannot tell "writing 100k events, healthy" from "wedged on a
    lock". Nothing is returned in that mode because nothing was captured.

    A timeout is NOT a clean failure to let escape as a traceback — it is the
    single most likely way a long leg dies — so it is re-raised as a `RigError`
    carrying whatever the child had already produced.
    """
    try:
        if stream:
            proc = subprocess.run(cmd, env=env, text=True, timeout=timeout, check=False)
            combined = "(streamed to this terminal, not captured)"
        else:
            proc = subprocess.run(
                cmd, env=env, capture_output=True, text=True, timeout=timeout, check=False
            )
            combined = proc.stdout + proc.stderr
    except subprocess.TimeoutExpired as exc:
        partial = _decode(exc.stdout) + _decode(exc.stderr)
        raise RigError(
            f"{what} did not finish within {timeout}s and was killed. That is a rig "
            f"failure, not a measurement. Partial output:\n{partial}"
        ) from exc
    if proc.returncode != 0:
        raise RigError(f"{what} failed (exit {proc.returncode}):\n{combined}")
    return combined


def _decode(raw: str | bytes | None) -> str:
    """Best-effort text from a `TimeoutExpired`, whose streams may be bytes or None."""
    if raw is None:
        return ""
    return raw if isinstance(raw, str) else raw.decode("utf-8", errors="replace")


def psql(host: str, port: int, dbname: str, sql: str) -> None:
    """Run one statement as the current user, for database setup only.

    Raises `RigError` carrying **PostgreSQL's own message**, not a bare exit
    status. That distinction is the whole point of this wrapper: the failure that
    actually happens here is `DROP DATABASE` refused because a leaked child still
    holds a connection, and the line that says so is `DETAIL: There is 1 other
    session using the database`. `subprocess.run(check=True, capture_output=True)`
    captures that line and then raises an exception whose text is only
    "returned non-zero exit status 1" — the rig would be holding the diagnosis and
    printing the riddle, which is the same wrong-cause-of-death failure the pty
    child leak produced.
    """
    try:
        proc = subprocess.run(
            ["psql", "-v", "ON_ERROR_STOP=1", "-h", host, "-p", str(port),
             "-d", dbname, "-c", sql],
            capture_output=True,
            text=True,
            timeout=PSQL_TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise RigError(
            f"psql timed out after {PSQL_TIMEOUT_SECONDS}s on {dbname}: {sql}\n"
            "A `CREATE`/`DROP DATABASE` that blocks this long is waiting on a lock "
            "held by a connection the rig failed to close."
        ) from exc
    if proc.returncode != 0:
        raise RigError(
            f"psql failed (exit {proc.returncode}) on {dbname}: {sql}\n"
            f"{proc.stdout}{proc.stderr}"
        )


def fresh_database(host: str, port: int, name: str) -> None:
    """Drop and recreate `name` with `cairn_pgx` installed."""
    psql(host, port, "postgres", f'DROP DATABASE IF EXISTS "{name}"')
    psql(host, port, "postgres", f'CREATE DATABASE "{name}"')
    psql(host, port, name, "CREATE EXTENSION IF NOT EXISTS cairn_pgx")


#: The substring that identifies `rpassword`'s OWN prompt, lower-cased.
#:
#: Deliberately NOT the looser "recovery code". `restore` prints the NEW node's
#: shown-once banner ("=== RECOVERY CODE — shown ONCE ...") several steps EARLIER,
#: before the node plane is even applied, and it prints a further banner reading
#: "Enter the OLD node's recovery code to unseal it:" before handing over to
#: `rpassword`. Matching "recovery code" fired on the first of those, so the rig
#: typed its answer into the terminal minutes before anything was reading, and it
#: worked only as type-ahead sitting in the line-discipline buffer — which in turn
#: worked only because the pinned `rpassword` restores terminal settings with
#: `TCSANOW`. A version using a flushing mode would have discarded the answer and
#: turned this measurement into a silent hour-long hang.
#:
#: "old recovery code" appears in `rpassword`'s two prompts ("old recovery code: "
#: and "old recovery code (try again): ") and in NEITHER banner — the second says
#: "old node's recovery code", which does not contain it.
_PROMPT_MARKER = "old recovery code"


def _redact(text: str, secret: str) -> str:
    """Blank a secret out of a transcript before it is printed.

    `rpassword` prints its prompt BEFORE it disables echo, so there is a narrow
    window in which the answer the rig types is echoed back by the terminal and
    lands in the captured transcript. Every rig error prints that transcript, so
    without this a failed run can put a live recovery code on the console.
    """
    return text.replace(secret, "<recovery code redacted>") if secret else text


def restore_under_pty(
    cmd: list[str], env: dict[str, str], recovery_code: str
) -> tuple[str, float, int]:
    """Run `cairn-node restore` on a pseudo-terminal, answering its prompts.

    Returns the combined output, the wall-clock seconds, and the exit status.

    **Why a pty and not a pipe.** `rpassword::prompt_password` opens `/dev/tty`
    and fails with *"Device not configured"* on anything else (that wording is
    macOS's ENXIO; other platforms word it differently), so a piped recovery code
    is not merely ignored — the read errors, the export never opens, and the
    restore completes having applied **zero** clinical records while exiting
    non-zero. That outcome is fast, which is why the rig checks what was recovered
    before it records a time.

    **Why it answers more than once.** `restore` allows `RECOVERY_CODE_ATTEMPTS`
    tries and re-prompts after a failure. A rig that answers once and then stops
    listening would, on any lost or mistimed first answer, sit until the ceiling
    and report "exceeded the rig's ceiling" — indistinguishable from a slow
    restore, which is the one thing this rig exists to measure.

    The clock starts before `fork` and stops after the child is reaped, so
    process start and teardown are inside the measurement. That is the honest
    boundary: the operator's stopwatch starts when they press return.
    """
    started = time.perf_counter()
    reaped = False
    status = 0
    child_pid, master_fd = pty.fork()
    if child_pid == 0:  # child
        try:
            os.execvpe(cmd[0], cmd, env)
        except BaseException:  # noqa: BLE001 — deliberate; see below
            # `execvpe` RAISES on failure, it does not return. Without this the
            # exception unwinds the forked COPY of the parent interpreter, which
            # runs the parent's cleanup and flushes its inherited stdout buffer —
            # re-emitting output the parent already printed and exiting 1 rather
            # than saying "could not exec".
            os._exit(127)
        os._exit(127)  # genuinely unreachable: a successful exec replaced the image

    chunks: list[str] = []
    answers_sent = 0
    prompts_seen = 0
    carry = ""
    deadline = started + SUBPROCESS_TIMEOUT_SECONDS
    prompt_deadline = started + PROMPT_TIMEOUT_SECONDS

    def transcript() -> str:
        return _redact("".join(chunks), recovery_code)

    try:
        while True:
            if time.perf_counter() > deadline:
                raise RigError(
                    "restore exceeded the rig's ceiling; aborting. Last output:\n"
                    + transcript()[-4000:]
                )
            ready, _, _ = select.select([master_fd], [], [], 1.0)
            if ready:
                try:
                    data = os.read(master_fd, 65536)
                except OSError as exc:
                    # EIO is the normal pty EOF: the last slave closed, so the
                    # child's stdio is gone. Anything else is NOT end-of-output,
                    # and treating it as one would silently truncate the very
                    # transcript completeness is judged from.
                    if exc.errno != errno.EIO:
                        raise RigError(
                            f"reading the restore's pty failed ({exc}); the "
                            "transcript is incomplete, so no timing can be "
                            "recorded. Partial output:\n" + transcript()[-4000:]
                        ) from exc
                    break
                if not data:
                    break
                text = data.decode("utf-8", errors="replace")
                chunks.append(text)
                # Count prompts INCREMENTALLY, and only while another answer could
                # still be owed. Re-scanning the whole transcript on every read
                # would be quadratic in the output size, and this loop is inside
                # the measured leg — the rig would be timing its own bookkeeping.
                if answers_sent < RECOVERY_CODE_ATTEMPTS:
                    window = (carry + text).lower()
                    prompts_seen += window.count(_PROMPT_MARKER)
                    # Carry one character less than the marker, so a marker split
                    # across two reads is still seen and none can be counted twice.
                    carry = text[-(len(_PROMPT_MARKER) - 1):]
            # Answer once per prompt actually seen, up to the number of tries the
            # command allows. Counting prompts rather than setting a one-shot flag
            # is what keeps a re-prompt answerable.
            while answers_sent < min(prompts_seen, RECOVERY_CODE_ATTEMPTS):
                os.write(master_fd, (recovery_code + "\n").encode())
                answers_sent += 1
            # The prompt deadline stays armed until the FIRST answer goes out; a
            # restore that never asks is a different failure from a slow one.
            if answers_sent == 0 and time.perf_counter() > prompt_deadline:
                raise RigError(
                    "the restore never asked for a recovery code. Output:\n"
                    + transcript()[-4000:]
                )
        # Reaping is inside the ceiling too: pty EOF means the child's stdio
        # closed, NOT that the process exited, so a blocking `waitpid` here could
        # hang past the ceiling that exists to stop exactly that.
        while True:
            pid, status = os.waitpid(child_pid, os.WNOHANG)
            if pid:
                reaped = True
                break
            if time.perf_counter() > deadline:
                raise RigError(
                    "the restore closed its pty but never exited; aborting. "
                    "Output:\n" + transcript()[-4000:]
                )
            time.sleep(0.05)
    finally:
        # Kill BEFORE closing the master. A rig error inside the loop — a wedged
        # restore, or a prompt that never came — leaves the child ALIVE holding a
        # database connection and, on the restore path, a half-populated database.
        # The next size in the curve then fails to `DROP DATABASE` because a
        # connection is still attached, and the run dies reporting the wrong cause.
        # If `os.close` ran first and raised, the kill would be skipped entirely.
        if not reaped:
            try:
                os.kill(child_pid, signal.SIGKILL)
                os.waitpid(child_pid, 0)
            except ProcessLookupError:
                pass  # ESRCH, and only ESRCH, means "already gone"
            except OSError as exc:
                # EPERM means the kill did NOT happen and the child still holds
                # the destination database. Say so here rather than letting the
                # next `DROP DATABASE` be the first sign of it.
                print(
                    f"  WARNING: could not reap the restore child {child_pid}: "
                    f"{exc}. It may still hold a database connection.",
                    file=sys.stderr,
                )
        try:
            os.close(master_fd)
        except OSError:
            pass  # the child closing the pty can already have invalidated it
    elapsed = time.perf_counter() - started
    return transcript(), elapsed, os.waitstatus_to_exitcode(status)


def measure_one(args: argparse.Namespace, patients: int) -> Measurement:
    """Seed, back up, and restore one corpus; return the point on the curve."""
    src, dst = f"{args.db_prefix}_src", f"{args.db_prefix}_dst"
    medium_dir = Path(args.workdir) / f"medium-{patients}"
    node_key = Path(args.workdir) / f"node-{patients}.key"
    restored_key = Path(args.workdir) / f"restored-{patients}.key"
    for stray in (node_key, restored_key):
        for suffix in ("", ".lsk", ".unwrap"):
            Path(str(stray) + suffix).unlink(missing_ok=True)
    if medium_dir.exists():
        for f in medium_dir.iterdir():
            f.unlink()
    medium_dir.mkdir(parents=True, exist_ok=True)
    medium = medium_dir / "medium.cairnb"

    user = os.environ.get("USER") or os.environ.get("LOGNAME") or getpass.getuser()
    src_conn = f"host={args.host} port={args.port} user={user} dbname={src}"
    dst_conn = f"host={args.host} port={args.port} user={user} dbname={dst}"
    op_env = dict(os.environ, CAIRN_KEY_PASSPHRASE=args.passphrase)

    print(f"\n=== {patients} patients ===", flush=True)
    fresh_database(args.host, args.port, src)

    # 1. Provision through the REAL ceremony: it mints the sealed key, the
    #    local-state escrow and the unwrap key, and prints the recovery code the
    #    restore will ask for. A `--insecure-plaintext` node has no escrow, so
    #    `backup` would write no sealed export and the restore would measure the
    #    degraded, custody-less path.
    init_out = run(
        [args.binary, "--conn", src_conn, "--key", str(node_key), "init",
         "--name", "measure-rig", "--address", "127.0.0.1:0"],
        op_env, "init",
    )
    recovery_code = parse_recovery_code(init_out)

    # 2. One real registration, to enroll the node's own `device` actor. That is
    #    an owner ceremony living in the CLI (`ensure_registration_actor`), and
    #    re-spelling it in the seeder would be a second copy of a ceremony — the
    #    mirror-list defect class this repo keeps paying for. Cheaper to run the
    #    shipped command once.
    run(
        [args.binary, "--conn", src_conn, "--key", str(node_key), "patient-register",
         "--name", "Seed Warmup", "--birth-date", "1970-01-01"],
        op_env, "warm-up registration",
    )

    # 3. Seed. NOT measured against the budget — only reported, because it also
    #    tells us whether a bigger corpus is reachable at all.
    #    It STREAMS, and keeps the seeder's progress lines. This is the longest leg
    #    by far, and captured output plus `--progress-every 0` meant it printed
    #    nothing at all for its whole duration — an operator watching a run could
    #    not tell healthy work from a wedged lock.
    seed_started = time.perf_counter()
    run(
        [args.seeder, "--conn", src_conn, "--key", str(node_key),
         "--patients", str(patients), "--meds-per-patient", str(args.meds_per_patient),
         "--progress-every", str(args.progress_every)],
        op_env, "seed", timeout=SEED_TIMEOUT_SECONDS, stream=True,
    )
    seed_s = time.perf_counter() - seed_started

    # 4. Capture the medium through the real command (#552 watches this number).
    backup_started = time.perf_counter()
    run(
        [args.binary, "--conn", src_conn, "--key", str(node_key), "backup",
         "--to", str(medium)],
        op_env, "backup",
    )
    backup_s = time.perf_counter() - backup_started

    # 5. The measured leg.
    fresh_database(args.host, args.port, dst)
    restore_env = dict(os.environ, CAIRN_KEY_PASSPHRASE=args.new_passphrase)
    out, restore_s, code = restore_under_pty(
        [args.binary, "--conn", dst_conn, "--key", str(restored_key), "restore",
         "--from", str(medium)],
        restore_env, recovery_code,
    )
    # The exit status is checked FIRST, and it is strictly broader than the
    # summary. `restore` deliberately prints its whole report and only THEN
    # fails ("Only NOW may the process fail", main.rs) — so a refused local-state
    # bundle, which means the dead node's custody was NOT installed, arrives as a
    # clean-looking clinical line followed by a non-zero exit. Reading only the
    # summary would record a passing time for the degraded, custody-less path this
    # rig provisions a sealed node specifically to avoid measuring.
    if code != 0:
        raise RigError(
            f"the restore exited {code}. Its summary line is printed before it "
            "fails, so a complete-looking clinical count is not proof of a "
            "complete ceremony. Not recording a timing.\nOutput:\n" + out
        )
    summary = parse_clinical_summary(out)
    if not summary.is_complete:
        raise RigError(
            "the restore did not recover the whole medium, so its timing is not a "
            f"measurement of the budget: {summary}. Output:\n{out}"
        )
    print(
        f"  seed {seed_s:.1f}s · backup {backup_s:.1f}s · "
        f"restore {restore_s:.1f}s · {summary.applied} applied",
        flush=True,
    )
    return Measurement(
        seed_s=seed_s,
        backup_s=backup_s,
        restore_s=restore_s,
        applied=summary.applied,
        on_medium=summary.on_medium,
    )


def discover_cluster(explicit_host: str | None, explicit_port: int | None) -> tuple[str, int]:
    """Resolve which PostgreSQL cluster to measure against, never assuming one.

    Delegates to `scripts/pg-target.sh`, which exists because a hardcoded 5532 is
    "true of exactly one machine in the world" (its own words) and because a
    cluster below the schema's version floor fails twenty-odd migrations deep with
    an error that names nothing. This rig cited that script in its docstring while
    defaulting to 5532 anyway, which is the assumption the script was written to
    abolish.

    An explicit `--port` still wins: an operator who names a server gets that
    server. The script's contract is one line, `<host> <port>`, on stdout.
    """
    if explicit_port is not None:
        return explicit_host or "127.0.0.1", explicit_port
    script = Path(__file__).resolve().parent / "pg-target.sh"
    try:
        proc = subprocess.run(
            ["bash", str(script)],
            capture_output=True,
            text=True,
            timeout=PSQL_TIMEOUT_SECONDS,
            check=False,
        )
    except OSError as exc:
        raise RigError(f"could not run {script}: {exc}") from exc
    if proc.returncode != 0:
        raise RigError(
            "no usable PostgreSQL cluster found, so there is nothing honest to "
            f"measure against. {script} said:\n{proc.stderr}"
        )
    parts = proc.stdout.split()
    if len(parts) != 2:
        raise RigError(
            f"{script} should print `<host> <port>`; it printed {proc.stdout!r}"
        )
    host, port = parts
    return (explicit_host or host), int(port)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    # No default port. `pg-target.sh` discovers the cluster and checks its version
    # floor; passing --port names one explicitly and skips discovery.
    parser.add_argument("--host", default=None)
    parser.add_argument("--port", type=int, default=None)
    parser.add_argument(
        "--sizes",
        default=DEFAULT_SIZES,
        help="comma-separated PATIENT counts (not event counts)",
    )
    parser.add_argument("--meds-per-patient", type=int, default=DEFAULT_MEDS_PER_PATIENT)
    parser.add_argument("--db-prefix", default="cairn_dr_measure")
    parser.add_argument("--workdir", default="/tmp/cairn-dr-measure")
    parser.add_argument("--binary", default="target/release/cairn-node")
    parser.add_argument("--seeder", default="target/release/examples/seed_measurement_corpus")
    parser.add_argument("--progress-every", type=int, default=250,
                        help="seeder progress line every N patients (0 disables)")
    # Passphrases are GENERATED per run, never defaulted to a literal. House rule 6:
    # a constant flowing into a binding named `passphrase` is cryptographic material
    # written down, and these seal a real node key. The rig mints a throwaway node
    # per size and needs no stable value, so there is nothing to trade away.
    parser.add_argument("--passphrase", default=None)
    parser.add_argument("--new-passphrase", default=None)
    parser.add_argument("--self-test", action="store_true",
                        help="exercise the parsers and exit; needs no database")
    args = parser.parse_args()
    args.passphrase = args.passphrase or secrets.token_hex(16)
    args.new_passphrase = args.new_passphrase or secrets.token_hex(16)

    if args.self_test:
        import unittest
        sys.argv = [sys.argv[0]]
        tests = unittest.defaultTestLoader.discover(
            str(Path(__file__).resolve().parent / "tests"),
            pattern="measure_dr_restore_test.py",
        )
        # An empty run reports SUCCESS: `TestResult.wasSuccessful()` is true when
        # nothing ran. A self-test that passes by finding no tests is worse than no
        # self-test, because it certifies the parsers that decide what number gets
        # written into a dated results file.
        if tests.countTestCases() == 0:
            raise RigError(
                "--self-test discovered no tests; expected "
                "scripts/tests/measure_dr_restore_test.py"
            )
        return 0 if unittest.TextTestRunner(verbosity=2).run(tests).wasSuccessful() else 1

    args.host, args.port = discover_cluster(args.host, args.port)
    print(f"measuring against PostgreSQL at {args.host}:{args.port}", flush=True)
    Path(args.workdir).mkdir(parents=True, exist_ok=True)
    rows = [measure_one(args, int(s)) for s in args.sizes.split(",")]
    print("\n" + format_results_table(rows))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (RigError, subprocess.SubprocessError, OSError) as exc:
        # `RigError` is the deliberate channel, but it was the ONLY one caught, so
        # every other real failure mode — a psql exit, a timeout, a missing binary,
        # an unwritable workdir — surfaced as a traceback instead of a diagnosis.
        print(f"rig error: {type(exc).__name__}: {exc}", file=sys.stderr)
        sys.exit(2)
