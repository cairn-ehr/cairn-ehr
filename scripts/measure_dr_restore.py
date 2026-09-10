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

**The measured leg is the shipped CLI, deliberately.** Everything timed here is
`cairn-node restore` as an operator would run it. Timing a library call instead
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
environment variable for it, unlike the new key's passphrase. So a restore
cannot be scripted, piped or run from cron at all, and this rig has to allocate
a pty to drive one. That is a finding about the product, not a convenience of
the rig; see the issue filed from the measurement run.

# Usage

    python3 scripts/measure_dr_restore.py --sizes 100,1000,3000,6000

Sizes are **patient counts**; each patient yields 3 demographic events plus
`--meds-per-patient` born-sealed clinical events. Run `--self-test` to exercise
the parsers without a database.

Requires a PostgreSQL cluster with `cairn_pgx` (see `scripts/pg-target.sh`), and
release builds of `cairn-node` and the `seed_measurement_corpus` example.
"""

from __future__ import annotations

import argparse
import os
import pty
import re
import select
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

#: Ceiling on a single restore, so a wedged run fails the rig instead of hanging
#: a session. Comfortably above the budget it is measuring.
RESTORE_TIMEOUT_SECONDS = 3600


class RigError(RuntimeError):
    """The rig could not produce an honest number, and says why rather than guessing."""


# ---------------------------------------------------------------------------
# Pure parsing — the half that decides what number gets written down
# ---------------------------------------------------------------------------

#: A recovery code is seven dash-joined groups of alphanumerics, the last shorter
#: than the rest. Anchored on that shape rather than on the banner around it, so a
#: reworded banner does not silently stop the rig finding the code.
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
    for match in _RECOVERY_CODE.finditer(text):
        candidate = match.group(1)
        # The banner lines are also upper-case and dashed; the code is the only
        # match whose first group is a full five characters and which is not a
        # word from the banner.
        if "RECOVERY" not in candidate:
            return candidate
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
        """
        return self.refused == 0 and (self.applied + self.already_present) >= self.on_medium


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

    `on_medium` is the only event count kept: it is what the restore itself
    reported finding, so it cannot drift from what was measured. An independent
    `events` field derived from the seed parameters was removed after a test
    caught the two disagreeing — a second spelling of a count is a second thing
    that can be wrong, and this one would have been wrong in the results file.
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
    out = [
        "| Events on medium | Seed (s) | Backup (s) | **Restore (s)** | Applied | ≤ 10 min |",
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


def run(cmd: list[str], env: dict[str, str], what: str) -> str:
    """Run a command to completion, returning combined output, raising on failure."""
    proc = subprocess.run(
        cmd, env=env, capture_output=True, text=True, timeout=RESTORE_TIMEOUT_SECONDS
    )
    combined = proc.stdout + proc.stderr
    if proc.returncode != 0:
        raise RigError(f"{what} failed (exit {proc.returncode}):\n{combined}")
    return combined


def psql(port: int, dbname: str, sql: str) -> None:
    """Run one statement as the current user, for database create/drop only."""
    subprocess.run(
        ["psql", "-h", "127.0.0.1", "-p", str(port), "-d", dbname, "-c", sql],
        check=True,
        capture_output=True,
        text=True,
    )


def fresh_database(port: int, name: str) -> None:
    """Drop and recreate `name` with `cairn_pgx` installed."""
    psql(port, "postgres", f'DROP DATABASE IF EXISTS "{name}"')
    psql(port, "postgres", f'CREATE DATABASE "{name}"')
    psql(port, name, "CREATE EXTENSION IF NOT EXISTS cairn_pgx")


def restore_under_pty(
    cmd: list[str], env: dict[str, str], recovery_code: str
) -> tuple[str, float, int]:
    """Run `cairn-node restore` on a pseudo-terminal, answering its one prompt.

    Returns the combined output, the wall-clock seconds, and the exit status.

    **Why a pty and not a pipe.** `rpassword::prompt_password` opens `/dev/tty`
    and fails with *"Device not configured"* on anything else, so a piped
    recovery code is not merely ignored — the read errors, the export never
    opens, and the restore completes having applied **zero** clinical records
    while exiting non-zero. That outcome is fast, which is why the rig checks
    what was recovered before it records a time.

    The clock starts before `fork` and stops after the child is reaped, so
    process start and teardown are inside the measurement. That is the honest
    boundary: the operator's stopwatch starts when they press return.
    """
    started = time.perf_counter()
    child_pid, master_fd = pty.fork()
    if child_pid == 0:  # child
        os.execvpe(cmd[0], cmd, env)
        os._exit(127)  # unreachable; execvpe replaces the image

    chunks: list[str] = []
    answered = False
    deadline = started + RESTORE_TIMEOUT_SECONDS
    prompt_deadline = started + PROMPT_TIMEOUT_SECONDS
    try:
        while True:
            if time.perf_counter() > deadline:
                raise RigError("restore exceeded the rig's ceiling; aborting")
            ready, _, _ = select.select([master_fd], [], [], 1.0)
            if ready:
                try:
                    data = os.read(master_fd, 65536)
                except OSError:
                    break  # the child closed the pty: it has exited
                if not data:
                    break
                chunks.append(data.decode("utf-8", errors="replace"))
            # Answer the one prompt, once. Matching on the visible prompt rather
            # than a fixed delay means the rig does not care how long the node
            # plane took to apply first.
            if not answered and "recovery code" in "".join(chunks).lower():
                os.write(master_fd, (recovery_code + "\n").encode())
                answered = True
            if not answered and time.perf_counter() > prompt_deadline:
                raise RigError("the restore never asked for a recovery code")
        _, status = os.waitpid(child_pid, 0)
    finally:
        os.close(master_fd)
    elapsed = time.perf_counter() - started
    return "".join(chunks), elapsed, os.waitstatus_to_exitcode(status)


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

    src_conn = f"host=127.0.0.1 port={args.port} user={os.environ['USER']} dbname={src}"
    dst_conn = f"host=127.0.0.1 port={args.port} user={os.environ['USER']} dbname={dst}"
    op_env = dict(os.environ, CAIRN_KEY_PASSPHRASE=args.passphrase)

    print(f"\n=== {patients} patients ===", flush=True)
    fresh_database(args.port, src)

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
    seed_started = time.perf_counter()
    run(
        [args.seeder, "--conn", src_conn, "--key", str(node_key),
         "--patients", str(patients), "--meds-per-patient", str(args.meds_per_patient),
         "--progress-every", "0"],
        op_env, "seed",
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
    fresh_database(args.port, dst)
    restore_env = dict(os.environ, CAIRN_KEY_PASSPHRASE=args.new_passphrase)
    out, restore_s, code = restore_under_pty(
        [args.binary, "--conn", dst_conn, "--key", str(restored_key), "restore",
         "--from", str(medium)],
        restore_env, recovery_code,
    )
    summary = parse_clinical_summary(out)
    if not summary.is_complete:
        raise RigError(
            "the restore did not recover the whole medium, so its timing is not a "
            f"measurement of the budget: {summary}. Exit {code}. Output:\n{out}"
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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=5532)
    parser.add_argument("--sizes", default="100,1000,3000,6000",
                        help="comma-separated PATIENT counts (not event counts)")
    parser.add_argument("--meds-per-patient", type=int, default=17)
    parser.add_argument("--db-prefix", default="cairn_dr_measure")
    parser.add_argument("--workdir", default="/tmp/cairn-dr-measure")
    parser.add_argument("--binary", default="target/release/cairn-node")
    parser.add_argument("--seeder", default="target/release/examples/seed_measurement_corpus")
    parser.add_argument("--passphrase", default="measure-rig-op-passphrase")
    parser.add_argument("--new-passphrase", default="restored-node-op-passphrase")
    parser.add_argument("--self-test", action="store_true",
                        help="exercise the parsers and exit; needs no database")
    args = parser.parse_args()

    if args.self_test:
        import unittest
        sys.argv = [sys.argv[0]]
        tests = unittest.defaultTestLoader.discover(
            str(Path(__file__).resolve().parent / "tests"),
            pattern="measure_dr_restore_test.py",
        )
        return 0 if unittest.TextTestRunner(verbosity=2).run(tests).wasSuccessful() else 1

    Path(args.workdir).mkdir(parents=True, exist_ok=True)
    rows = [measure_one(args, int(s)) for s in args.sizes.split(",")]
    print("\n" + format_results_table(rows))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except RigError as exc:
        print(f"rig error: {exc}", file=sys.stderr)
        sys.exit(2)
