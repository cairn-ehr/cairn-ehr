#!/usr/bin/env python3
"""Load a SyntheticHealthData cohort into a Cairn node through the node's own doors (#1214).

SyntheticHealthData (a sibling project) writes an invented general-practice cohort as one
SQLite file, `cohort.sqlite`. This script reads that file and writes what Cairn has a door
for TODAY — patients and their medicines — by running `cairn-node` verbs, exactly as an
operator at a terminal would. It then reports what it had to leave behind and why, so every
run restates how much of a realistic record Cairn can hold.

# Why it drives the CLI and builds no events itself

Every other volume loader in this repository (`measure_patient_search.py`,
`bench-reproject.sh`, `db/bench/`) inserts rows straight into a projection or the log. That
is right for what they measure and it means none of them tests the doors. This one exists to
test them: each patient goes through the search-before-create funnel (ADR-0061) and each
medicine through `medication-assert`, signed by the node's key and checked by the in-database
floor. The price is one process per write, which is fine for hundreds of patients and is the
measured cost of having no bulk door (#1212).

# What it will not do

* **Read a file that does not say it is synthetic.** The cohort's identifiers are valid in
  form (an IHI, a Medicare number), so the marker inside the file is the only thing that
  tells it from a real extract. No marker, no import.
* **Write into a node that holds anyone else's charts.** Cairn cannot yet mark a chart or a
  node as test data (#1207). Until it can, this script refuses a node holding any chart that
  does not carry a cohort record number. This is a guard in a script, not a floor: it stops
  the honest mistake and nothing else. NEVER peer a node this has written to with a real one.
* **Register past a displayed candidate on its own.** When the funnel shows existing charts
  for a patient, a person is meant to look at them. By default the patient is HELD and
  counted. `--register-past-candidates` registers anyway; the registration then attests a
  list that no human read, which is acceptable on a test node and nowhere else.
* **Invent a door.** Sex, address, conditions, results, imaging reports and letters have no
  door or no stream (#1208 to #1213). They are counted and left in the file.

# Running it twice

A patient is recognised by the cohort's record number, recorded on the chart under a system
that names the cohort (`mrn-shd-<tag>`, where the tag is derived from the cohort's seed and
dates). A second run finds each chart by that number and writes only what is missing, so a
run that was interrupted can simply be run again.

# Usage

    python3 scripts/import_synthetic_cohort.py out/cohort1/cohort.sqlite --dry-run

    CAIRN_CONN='host=127.0.0.1 port=5532 user=postgres dbname=cairn_synth' \\
    CAIRN_KEY_PASSPHRASE=... \\
    python3 scripts/import_synthetic_cohort.py out/cohort1/cohort.sqlite \\
        --cairn-node target/debug/cairn-node --key node.key --report import-report.json

Exit status: 0 = everything with a door is on the node; 3 = INCOMPLETE (patients held or a
write failed; run again or read the report); 2 = REFUSED (nothing was written).

The connection is the database OWNER's, because that is the only login `cairn-node`'s
clinical verbs run under today: the runtime role (`cairn_node`) may not search, and an
advisory login (`cairn_agent`) may not read the node's identity or the actor registry
(measured 2026-11-28). The events still pass through `submit_event` and are checked by it;
what the owner's login does not prove is that an unprivileged client could have made them.

Requires only the Python standard library, `psql` on PATH (for the one read the guard makes)
and a provisioned node (`cairn-node init`). Pure functions are tested in
`scripts/tests/import_synthetic_cohort_test.py`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
from collections.abc import Callable, Sequence
from dataclasses import asdict, dataclass, field
from pathlib import Path

# ---------------------------------------------------------------------------------------
# The cohort file
# ---------------------------------------------------------------------------------------

# SyntheticHealthData stamps every cohort file with this SQLite `application_id` (the ASCII
# bytes "SHDC"). Its documentation says a reader should refuse a file without it.
SHD_APPLICATION_ID = 0x53484443
# The newest cohort format this script was written against. Formats only ever ADD tables and
# columns, so a newer file is still read; the report says it may hold kinds unknown here.
KNOWN_DATABASE_FORMAT = 3

# How SyntheticHealthData names an identifier system, and how Cairn spells it (ADR-0113:
# lower case, the issuing region after a hyphen).
IHI_URI = "http://ns.electronichealth.net.au/id/hi/ihi/1.0"
MEDICARE_URI = "http://ns.electronichealth.net.au/id/medicare-number"
MRN_URI = "urn:synthetichealthdata:patient"
CAIRN_SYSTEMS = {IHI_URI: "ihi-au", MEDICARE_URI: "medicare-au"}
# Every cohort record number is recorded under a system with this prefix. The guard reads it
# to tell this script's charts from anyone else's.
MRN_SYSTEM_PREFIX = "mrn-shd-"

DRUGREF_MOIETY = "drugref-moiety"


class CohortRefused(Exception):
    """The file is not a cohort this script may read. Nothing has been written."""


@dataclass(frozen=True)
class Medicine:
    n: int
    drug: str
    started: str
    stopped: str | None
    drugref: str | None


@dataclass(frozen=True)
class Patient:
    mrn: str
    given: str
    family: str
    birth_date: str
    identifiers: tuple[tuple[str, str], ...]
    """(system as the cohort names it, value), in the cohort's order."""
    medicines: tuple[Medicine, ...]


@dataclass(frozen=True)
class Cohort:
    spec: dict
    database_format: int
    patients: tuple[Patient, ...]
    left_behind: dict[str, int]
    """What the file holds that Cairn has no door for, by kind."""


def cohort_tag(spec: dict) -> str:
    """A short name for "these invented people", stable across runs of one cohort.

    Two cohorts drawn with different seeds reuse the same record numbers (SHD000001…) for
    different people, so the number alone must never identify a chart. The tag is a digest
    of everything that decides who the people are. `patients` is left out on purpose: the
    generator promises a larger cohort keeps every patient the smaller one had, so growing a
    cohort must find the charts already there, not register them again.
    """
    who = {k: v for k, v in spec.items() if k != "patients"}
    return hashlib.sha256(json.dumps(who, sort_keys=True).encode()).hexdigest()[:8]


def mrn_system(spec: dict) -> str:
    return MRN_SYSTEM_PREFIX + cohort_tag(spec)


def _count(db: sqlite3.Connection, sql: str) -> int:
    """A count from a table that an older cohort format may not have (0 then)."""
    try:
        return int(db.execute(sql).fetchone()[0])
    except sqlite3.OperationalError:
        return 0


def read_cohort(path: Path) -> Cohort:
    """Read the parts of a cohort file this script uses, refusing anything not marked synthetic.

    The file is opened read-only: a cohort is written once and never changed.
    """
    if not path.is_file():
        raise CohortRefused(f"{path} is not a file")
    db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    try:
        try:
            application_id = db.execute("PRAGMA application_id").fetchone()[0]
        except sqlite3.DatabaseError as e:
            raise CohortRefused(f"{path} is not an SQLite database") from e
        if application_id != SHD_APPLICATION_ID:
            raise CohortRefused(
                f"{path} does not carry SyntheticHealthData's marker (application_id "
                f"{application_id:#x}, expected {SHD_APPLICATION_ID:#x}). Its identifiers are "
                "valid in form, so a file without the marker is treated as real data."
            )
        try:
            row = db.execute(
                "SELECT provenance, spec, database_format FROM manifest WHERE id = 1"
            ).fetchone()
        except sqlite3.OperationalError as e:
            raise CohortRefused(f"{path} has no readable manifest") from e
        if row is None or row[0] != "synthetic":
            raise CohortRefused(f"{path}: the manifest does not say provenance = 'synthetic'")
        spec = json.loads(row[1])

        identifiers: dict[str, list[tuple[str, str]]] = {}
        for pid, system, value in db.execute(
            "SELECT patient_id, system, value FROM patient_identifier ORDER BY patient_id, n"
        ):
            identifiers.setdefault(pid, []).append((system, value))
        medicines: dict[str, list[Medicine]] = {}
        for pid, n, drug, started, stopped, drugref in db.execute(
            "SELECT patient_id, n, drug, started, stopped, drugref FROM medication "
            "ORDER BY patient_id, n"
        ):
            medicines.setdefault(pid, []).append(Medicine(n, drug, started, stopped, drugref))
        patients = tuple(
            Patient(
                mrn=pid,
                given=given,
                family=family,
                birth_date=birth_date,
                identifiers=tuple(identifiers.get(pid, ())),
                medicines=tuple(medicines.get(pid, ())),
            )
            for pid, given, family, birth_date in db.execute(
                "SELECT id, given, family, birth_date FROM patient ORDER BY id"
            )
        )
        left_behind = {
            "sex": len(patients),
            "address": len(patients),
            "conditions": _count(db, "SELECT count(*) FROM condition"),
            "pathology results": _count(db, "SELECT count(*) FROM result"),
            "progress notes": _count(
                db, "SELECT count(*) FROM document WHERE kind = 'progress_note'"
            ),
            "imaging reports": _count(
                db, "SELECT count(*) FROM document WHERE kind = 'imaging_report'"
            ),
            "specialist letters": _count(
                db, "SELECT count(*) FROM document WHERE kind = 'specialist_letter'"
            ),
            "clinicians": _count(db, "SELECT count(*) FROM clinician"),
        }
        return Cohort(spec, int(row[2]), patients, left_behind)
    finally:
        db.close()


# Why each kind is left behind, and the issue that tracks the missing door. Printed with the
# counts so the report explains itself to someone who has not read this file.
WHY_LEFT_BEHIND = {
    "sex": "no door records a stated sex (#1211)",
    "address": "no door records an address (#1211)",
    "conditions": "no problem-list stream (#1209)",
    "pathology results": "no results stream (#1208)",
    "progress notes": "notes are written only from the window (#1212)",
    "imaging reports": "no received-document stream (#1210)",
    "specialist letters": "no received-document stream (#1210)",
    "clinicians": "the entity directory is unbuilt (#1213)",
    "identifiers in a system Cairn has no spelling for": "ADR-0113 names no spelling",
    "drug codes that are not a drugref moiety": "the floor takes a moiety uuid only (ADR-0059)",
}


# ---------------------------------------------------------------------------------------
# From a cohort row to a `cairn-node` command line (all pure)
# ---------------------------------------------------------------------------------------

_UUID = r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
_UUID_ONLY = re.compile(rf"^{_UUID}$")
_REGISTERED = re.compile(rf"^registered patient ({_UUID})$", re.MULTILINE)
_RECORDED = re.compile(rf"^recorded medication for {_UUID}; thread ({_UUID});", re.MULTILINE)
_CANDIDATE = re.compile(rf"^({_UUID})  ", re.MULTILINE)


def cairn_identifiers(patient: Patient, system_for_mrn: str) -> tuple[list[str], int]:
    """The `system=value` arguments for a patient, and how many identifiers had no spelling.

    An identifier in a system this script cannot spell Cairn's way is DROPPED and counted,
    never passed through under a guessed name: a wrong system name makes two different
    schemes look like one, and a search would then join people on it.
    """
    args, unspelled = [], 0
    for system, value in patient.identifiers:
        if system == MRN_URI:
            args.append(f"{system_for_mrn}={value}")
        elif system in CAIRN_SYSTEMS:
            args.append(f"{CAIRN_SYSTEMS[system]}={value}")
        else:
            unspelled += 1
    return args, unspelled


def search_argv(patient: Patient, system_for_mrn: str) -> list[str]:
    """Find the chart that carries this cohort record number, and nothing looser."""
    return ["patient-search", "--identifier", f"{system_for_mrn}={patient.mrn}"]


def register_argv(patient: Patient, system_for_mrn: str, *, confirm_new: bool) -> list[str]:
    """Register through the funnel. The name is recorded as typed: given name, then family."""
    argv = [
        "patient-register",
        "--name",
        f"{patient.given} {patient.family}",
        "--birth-date",
        patient.birth_date,
    ]
    for identifier in cairn_identifiers(patient, system_for_mrn)[0]:
        argv += ["--identifier", identifier]
    if confirm_new:
        argv.append("--confirm-new")
    return argv


def usable_drugref(medicine: Medicine) -> str | None:
    """The medicine's drugref moiety, if it is one the floor will take (a uuid)."""
    code = medicine.drugref
    return code if code and _UUID_ONLY.match(code) else None


def medication_argv(chart: str, medicine: Medicine) -> list[str]:
    """Record a medicine the cohort's record says the patient takes.

    `external-record` is the honest source: this was read from another record, not heard
    from the patient or seen by a clinician here. No dose is given because the cohort's
    medication table holds none (SyntheticHealthData#20); the chart then shows it as unknown.
    """
    argv = [
        "medication-assert",
        chart,
        "--term",
        medicine.drug,
        "--info-source",
        "external-record",
        "--started",
        medicine.started,
        "--started-precision",
        "day",
    ]
    code = usable_drugref(medicine)
    if code:
        argv += [
            "--coding-system",
            DRUGREF_MOIETY,
            "--coding-code",
            code,
            "--coding-display",
            medicine.drug,
        ]
    return argv


def cease_argv(chart: str, thread: str, medicine: Medicine) -> list[str]:
    assert medicine.stopped is not None
    return [
        "medication-cease",
        chart,
        thread,
        "--stopped",
        medicine.stopped,
        "--stopped-precision",
        "day",
    ]


def parse_registered(stdout: str) -> str | None:
    m = _REGISTERED.search(stdout)
    return m.group(1) if m else None


def parse_recorded_thread(stdout: str) -> str | None:
    m = _RECORDED.search(stdout)
    return m.group(1) if m else None


def parse_candidates(stdout: str) -> list[str]:
    """The chart ids `patient-search` / `patient-register` printed, in order, without repeats."""
    seen: list[str] = []
    for chart in _CANDIDATE.findall(stdout):
        if chart not in seen:
            seen.append(chart)
    return seen


def already_recorded(rows: Sequence[dict], medicine: Medicine) -> bool:
    """Whether the chart's medication list already holds this medicine from an earlier run.

    Matched on the term and the start date together: a patient may have taken one drug twice.
    """
    return any(
        r.get("term") == medicine.drug and r.get("started_value") == medicine.started
        for r in rows
    )


# ---------------------------------------------------------------------------------------
# The run
# ---------------------------------------------------------------------------------------


@dataclass(frozen=True)
class Completed:
    """What one `cairn-node` invocation returned."""

    returncode: int
    stdout: str
    stderr: str


Run = Callable[[list[str]], Completed]
"""Runs one `cairn-node` verb. Injected, so the run is tested against a stand-in node."""


@dataclass
class Report:
    cohort_tag: str = ""
    mrn_system: str = ""
    patients_registered: int = 0
    patients_already_present: int = 0
    patients_held: int = 0
    patients_failed: int = 0
    medicines_recorded: int = 0
    medicines_already_present: int = 0
    medicines_ceased: int = 0
    medicines_failed: int = 0
    medicines_coded: int = 0
    register_seconds: float = 0.0
    medicine_seconds: float = 0.0
    held: list[dict] = field(default_factory=list)
    failures: list[dict] = field(default_factory=list)
    left_behind: dict[str, int] = field(default_factory=dict)
    newer_format: bool = False

    def incomplete(self) -> bool:
        return bool(self.patients_held or self.patients_failed or self.medicines_failed)


def _refusal(stderr: str) -> str:
    """Why a command failed, in one line.

    `cairn-node` prints `Error: <what>` and then, under `Caused by:`, the database's own
    `ERROR: <why>`; with RUST_BACKTRACE set it prints a stack after both. The cause is the
    useful half ("db error" alone says nothing), so both are joined and the stack is
    dropped. Kept short and never the whole stream: this goes into a report file, and on a
    synthetic node that is harmless but the habit should not be.
    """
    lines = [ln.strip() for ln in stderr.splitlines() if ln.strip()]
    what = next((ln for ln in lines if ln.startswith(("Error:", "error:"))), None)
    why = next((ln.split("ERROR:", 1)[1].strip() for ln in lines if "ERROR:" in ln), None)
    if what and why:
        return f"{what.split(':', 1)[1].strip()}: {why}"[:300]
    return (what or (lines[0] if lines else ""))[:300]


def _chart_for(patient: Patient, system_for_mrn: str, run: Run, opts: Options, report: Report):
    """Find or register the patient's chart. Returns (chart id or None, was it there before)."""
    found = run(search_argv(patient, system_for_mrn))
    if found.returncode != 0:
        report.patients_failed += 1
        report.failures.append(
            {"patient": patient.mrn, "step": "search", "why": _refusal(found.stderr)}
        )
        return None, False
    charts = parse_candidates(found.stdout)
    if len(charts) == 1:
        report.patients_already_present += 1
        return charts[0], True
    if len(charts) > 1:
        # Two charts carry one cohort number. Which is "the" chart is a human's decision
        # (link, or one is a mistake); writing medicines to either would be a guess.
        report.patients_held += 1
        report.held.append(
            {"patient": patient.mrn, "why": "more than one chart carries this record number",
             "charts": charts}
        )
        return None, False

    started = time.monotonic()
    done = run(register_argv(patient, system_for_mrn, confirm_new=False))
    if done.returncode != 0 and parse_candidates(done.stdout):
        if not opts.register_past_candidates:
            report.register_seconds += time.monotonic() - started
            report.patients_held += 1
            report.held.append(
                {"patient": patient.mrn, "why": "the funnel showed existing charts",
                 "charts": parse_candidates(done.stdout)}
            )
            return None, False
        done = run(register_argv(patient, system_for_mrn, confirm_new=True))
    report.register_seconds += time.monotonic() - started
    chart = parse_registered(done.stdout) if done.returncode == 0 else None
    if chart is None:
        report.patients_failed += 1
        report.failures.append(
            {"patient": patient.mrn, "step": "register", "why": _refusal(done.stderr)}
        )
        return None, False
    report.patients_registered += 1
    return chart, False


def _medicines(patient: Patient, chart: str, existed: bool, run: Run, report: Report) -> None:
    """Record the patient's medicines, skipping any an earlier run already wrote."""
    rows: Sequence[dict] = ()
    if existed and patient.medicines:
        listed = run(["medication-list", chart, "--json"])
        if listed.returncode != 0:
            # Without the list we cannot tell what is already there, and recording blind
            # would duplicate every line. Fail these medicines; the next run retries.
            report.medicines_failed += len(patient.medicines)
            report.failures.append(
                {"patient": patient.mrn, "step": "medication-list",
                 "why": _refusal(listed.stderr)}
            )
            return
        rows = json.loads(listed.stdout).get("rows", [])
    for medicine in patient.medicines:
        if already_recorded(rows, medicine):
            report.medicines_already_present += 1
            continue
        started = time.monotonic()
        done = run(medication_argv(chart, medicine))
        report.medicine_seconds += time.monotonic() - started
        thread = parse_recorded_thread(done.stdout) if done.returncode == 0 else None
        if thread is None:
            report.medicines_failed += 1
            report.failures.append(
                {"patient": patient.mrn, "step": f"medication {medicine.n}",
                 "why": _refusal(done.stderr)}
            )
            continue
        report.medicines_recorded += 1
        report.medicines_coded += usable_drugref(medicine) is not None
        if medicine.stopped:
            ceased = run(cease_argv(chart, thread, medicine))
            if ceased.returncode == 0:
                report.medicines_ceased += 1
            else:
                report.medicines_failed += 1
                report.failures.append(
                    {"patient": patient.mrn, "step": f"cease medication {medicine.n}",
                     "why": _refusal(ceased.stderr)}
                )


@dataclass(frozen=True)
class Options:
    register_past_candidates: bool = False
    limit: int | None = None


def import_cohort(cohort: Cohort, run: Run, opts: Options | None = None) -> Report:
    """Write every patient and medicine that has a door, and account for the rest."""
    opts = opts or Options()
    system_for_mrn = mrn_system(cohort.spec)
    report = Report(
        cohort_tag=cohort_tag(cohort.spec),
        mrn_system=system_for_mrn,
        left_behind=dict(cohort.left_behind),
        newer_format=cohort.database_format > KNOWN_DATABASE_FORMAT,
    )
    patients = cohort.patients[: opts.limit] if opts.limit is not None else cohort.patients
    unspelled = sum(cairn_identifiers(p, system_for_mrn)[1] for p in patients)
    uncodable = sum(
        1 for p in patients for m in p.medicines if m.drugref and not usable_drugref(m)
    )
    if unspelled:
        report.left_behind["identifiers in a system Cairn has no spelling for"] = unspelled
    if uncodable:
        report.left_behind["drug codes that are not a drugref moiety"] = uncodable
    for patient in patients:
        chart, existed = _chart_for(patient, system_for_mrn, run, opts, report)
        if chart is not None:
            _medicines(patient, chart, existed, run, report)
    return report


# ---------------------------------------------------------------------------------------
# The guard, the real runner, and the command line
# ---------------------------------------------------------------------------------------

# Charts on the node that carry no cohort record number: anyone's but ours. Read from the
# projections with psql because no `cairn-node` verb lists every chart. `patient_demographic`
# and `patient_identifier` are the two projections a registration fills.
FOREIGN_CHARTS_SQL = f"""
SELECT count(*) FROM (
    SELECT patient_id FROM patient_demographic
    UNION
    SELECT patient_id FROM patient_identifier
) AS chart
WHERE NOT EXISTS (
    SELECT 1 FROM patient_identifier i
     WHERE i.patient_id = chart.patient_id AND i.system LIKE '{MRN_SYSTEM_PREFIX}%'
)
"""


def foreign_chart_count(conn: str) -> int:
    """How many charts on the node were not written by this script. Raises if it cannot ask."""
    out = subprocess.run(
        ["psql", conn, "-X", "-At", "-v", "ON_ERROR_STOP=1", "-c", FOREIGN_CHARTS_SQL],
        capture_output=True, text=True, check=False,
    )
    if out.returncode != 0:
        raise CohortRefused(
            "could not ask the node which charts it holds, so cannot tell whether it is safe "
            f"to write: {_refusal(out.stderr)}"
        )
    return int(out.stdout.strip())


def node_runner(cairn_node: str, key: str, conn: str) -> Run:
    """Run real `cairn-node` verbs. The connection string goes in the environment, never on
    the command line, because it can carry a password and a command line is world-readable."""
    env = dict(os.environ, CAIRN_CONN=conn)

    def run(argv: list[str]) -> Completed:
        done = subprocess.run(
            [cairn_node, "--key", key, *argv],
            capture_output=True, text=True, check=False, env=env, stdin=subprocess.DEVNULL,
        )
        return Completed(done.returncode, done.stdout, done.stderr)

    return run


def report_lines(report: Report, patients_in_file: int) -> list[str]:
    """The run, as a person reads it."""
    lines = [
        f"cohort {report.cohort_tag} ({patients_in_file} patients in the file), "
        f"record numbers under {report.mrn_system}",
        "",
        f"patients   registered {report.patients_registered}, already present "
        f"{report.patients_already_present}, HELD {report.patients_held}, "
        f"failed {report.patients_failed}",
        f"medicines  recorded {report.medicines_recorded} ({report.medicines_coded} coded), "
        f"already present {report.medicines_already_present}, ceased "
        f"{report.medicines_ceased}, failed {report.medicines_failed}",
    ]
    if report.patients_registered:
        each = report.register_seconds / report.patients_registered
        lines.append(f"time       {each:.2f} s per registration")
    if report.medicines_recorded:
        each = report.medicine_seconds / report.medicines_recorded
        lines.append(f"           {each:.2f} s per medicine")
    lines += ["", "left in the file (Cairn has no door for these yet):"]
    for kind, count in report.left_behind.items():
        if count:
            lines.append(f"  {count:>7}  {kind}: {WHY_LEFT_BEHIND.get(kind, '')}")
    if report.newer_format:
        lines.append(
            "  the file is in a newer format than this script knows, and may hold further kinds"
        )
    for held in report.held[:10]:
        lines.append(f"held    {held['patient']}: {held['why']} ({', '.join(held['charts'])})")
    for failure in report.failures[:10]:
        lines.append(f"failed  {failure['patient']} at {failure['step']}: {failure['why']}")
    hidden = max(len(report.held) - 10, 0) + max(len(report.failures) - 10, 0)
    if hidden:
        lines.append(f"… and {hidden} more; --report FILE writes them all")
    return lines


def main(argv: Sequence[str] | None = None) -> int:
    p = argparse.ArgumentParser(
        description="Load a SyntheticHealthData cohort into a Cairn TEST node through "
        "cairn-node's own verbs, and report what could not be loaded."
    )
    p.add_argument("cohort", type=Path, help="a cohort.sqlite written by SyntheticHealthData")
    p.add_argument("--cairn-node", default="cairn-node", help="the cairn-node binary")
    p.add_argument("--key", default="node.key", help="the node's key file")
    p.add_argument("--conn", default=os.environ.get("CAIRN_CONN"),
                   help="the node's connection string (default: CAIRN_CONN)")
    p.add_argument("--dry-run", action="store_true",
                   help="read the cohort and say what would be loaded; touch no node")
    p.add_argument("--limit", type=int, help="load only the first N patients")
    p.add_argument("--register-past-candidates", action="store_true",
                   help="register a patient even when the funnel shows existing charts; the "
                   "registration then attests a list no person read (test nodes only)")
    p.add_argument("--report", type=Path, help="also write the full report as JSON")
    args = p.parse_args(argv)

    try:
        cohort = read_cohort(args.cohort)
        if args.dry_run:
            patients = cohort.patients[: args.limit] if args.limit else cohort.patients
            medicines = sum(len(x.medicines) for x in patients)
            print(f"would load {len(patients)} patients and {medicines} medicines "
                  f"under {mrn_system(cohort.spec)}")
            for kind, count in cohort.left_behind.items():
                print(f"  would leave {count:>7}  {kind}: {WHY_LEFT_BEHIND[kind]}")
            return 0
        if not args.conn:
            raise CohortRefused("no connection: pass --conn or set CAIRN_CONN")
        foreign = foreign_chart_count(args.conn)
        if foreign:
            raise CohortRefused(
                f"the node holds {foreign} chart(s) that no cohort import created. Synthetic "
                "patients are never written beside others: Cairn cannot yet mark them as "
                "invented (#1207). Use a node kept for test data."
            )
    except CohortRefused as e:
        print(f"REFUSED: {e}", file=sys.stderr)
        return 2

    run = node_runner(args.cairn_node, args.key, args.conn)
    report = import_cohort(
        cohort, run, Options(args.register_past_candidates, args.limit)
    )
    print("\n".join(report_lines(report, len(cohort.patients))))
    if args.report:
        args.report.write_text(json.dumps(asdict(report), indent=2) + "\n")
    if report.incomplete():
        print("INCOMPLETE: see the held and failed lines above.", file=sys.stderr)
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
