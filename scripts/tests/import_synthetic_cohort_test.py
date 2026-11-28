#!/usr/bin/env python3
"""Tests for `scripts/import_synthetic_cohort.py`, without a database or a `cairn-node` binary.

The importer's whole job is to be trusted about three things: that it only reads a file
marked synthetic, that running it twice writes nothing twice, and that it never walks past
the registration funnel on its own. Each is tested here against a stand-in node (`FakeNode`)
that answers the handful of `cairn-node` verbs the importer runs, in the exact text the real
binary prints.

What is NOT tested here: that the real binary still prints that text. A run against a real
node shows that, and its figures are recorded in the PR that cites them.

    python3 scripts/tests/import_synthetic_cohort_test.py
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sqlite3
import sys
import tempfile
import uuid

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "import_synthetic_cohort.py"

WARFARIN_MOIETY = "69b0b39c-a2f7-5b36-8983-37d97c823990"


def load_script():
    """Import the importer by path — it is a script, not an installed module."""
    spec = importlib.util.spec_from_file_location("import_synthetic_cohort", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    # Registered before execution so `@dataclass` can resolve the module it is defined in.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


# ---------------------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------------------

SPEC = {"seed": 1, "patients": 2, "as_of": "2026-11-01", "history_years": 3.0,
        "chronic_share": 0.8}

PEOPLE = [
    # mrn, given, family, birth_date, [(drug, started, stopped, drugref)]
    ("SHD000001", "Biwei", "Chipp", "1955-07-15",
     [("perindopril", "2019-03-02", None, None),
      ("warfarin", "2021-06-10", "2024-01-05", WARFARIN_MOIETY)]),
    ("SHD000002", "Istvan", "Kiseli", "1952-03-15", [("metformin", "2015-01-20", None, None)]),
]


def write_cohort(directory, *, application_id=None, provenance="synthetic", spec=SPEC,
                 people=PEOPLE, extra_identifier=None, database_format=3):
    """A cohort file with the tables the importer reads, in SyntheticHealthData's shape.

    Built by hand so this test needs no sibling checkout. The real file's `manifest` has a
    CHECK that provenance is 'synthetic'; this one leaves it out so the refusal can be tested.
    """
    m = load_script()
    path = pathlib.Path(directory) / "cohort.sqlite"
    db = sqlite3.connect(path)
    app = m.SHD_APPLICATION_ID if application_id is None else application_id
    db.execute(f"PRAGMA application_id = {app}")
    db.executescript(
        """
        CREATE TABLE manifest (id INTEGER PRIMARY KEY, database_format INTEGER,
                               provenance TEXT, spec TEXT);
        CREATE TABLE patient (id TEXT PRIMARY KEY, family TEXT, given TEXT, sex TEXT,
                              birth_date TEXT);
        CREATE TABLE patient_identifier (patient_id TEXT, n INTEGER, system TEXT, value TEXT);
        CREATE TABLE medication (patient_id TEXT, n INTEGER, drug TEXT, started TEXT,
                                 stopped TEXT, drugref TEXT);
        CREATE TABLE condition (patient_id TEXT, n INTEGER, label TEXT);
        CREATE TABLE result (collection_id TEXT, n INTEGER);
        CREATE TABLE document (id TEXT, kind TEXT);
        CREATE TABLE clinician (id INTEGER PRIMARY KEY);
        """
    )
    db.execute("INSERT INTO manifest VALUES (1, ?, ?, ?)",
               (database_format, provenance, json.dumps(spec)))
    for i, (mrn, given, family, born, medicines) in enumerate(people):
        db.execute("INSERT INTO patient VALUES (?, ?, ?, 'female', ?)",
                   (mrn, family, given, born))
        # The seed is in the numbers because another seed's people are other people: only
        # the record number (SHD000001…) is reused across cohorts, as in the real generator.
        seed = spec["seed"]
        identifiers = [(m.MRN_URI, mrn), (m.IHI_URI, f"800360000000{seed:02d}{i:02d}"),
                       (m.MEDICARE_URI, f"20016275{seed:02d}{i}")]
        if extra_identifier:
            identifiers.append(extra_identifier)
        for n, (system, value) in enumerate(identifiers):
            db.execute("INSERT INTO patient_identifier VALUES (?, ?, ?, ?)",
                       (mrn, n, system, value))
        for n, (drug, started, stopped, drugref) in enumerate(medicines):
            db.execute("INSERT INTO medication VALUES (?, ?, ?, ?, ?, ?)",
                       (mrn, n, drug, started, stopped, drugref))
    db.executemany("INSERT INTO condition VALUES ('SHD000001', ?, 'Hypertension')",
                   [(n,) for n in range(3)])
    db.executemany("INSERT INTO result VALUES ('c', ?)", [(n,) for n in range(7)])
    db.executemany("INSERT INTO document VALUES (?, ?)",
                   [("d1", "progress_note"), ("d2", "imaging_report"),
                    ("d3", "specialist_letter"), ("d4", "specialist_letter")])
    db.execute("INSERT INTO clinician VALUES (1)")
    db.commit()
    db.close()
    return path


class FakeNode:
    """A stand-in for `cairn-node`: it keeps charts and medicines and prints what the real
    binary prints (`registered_patient_lines`, `recorded_medication_line`, the candidate
    table), so the importer's parsing is exercised against the real text shapes."""

    def __init__(self, m):
        self.m = m
        self.charts: dict[str, dict] = {}  # chart id -> {name, identifiers, medicines}
        self.calls: list[list[str]] = []
        self.fail_medication_after: int | None = None  # simulate a crash part-way

    # -- helpers -------------------------------------------------------------------------
    def _flag(self, argv, name):
        return [argv[i + 1] for i, a in enumerate(argv) if a == name]

    def _table(self, charts):
        if not charts:
            return "no candidates found\n"
        head = f"{'patient_id':<36}  {'name':<28}  {'age':>4}  trust\n"
        return head + "".join(
            f"{c:<36}  {self.charts[c]['name']:<28}  {'70':>4}  ok\n" for c in charts
        )

    def _matching(self, argv):
        """Charts sharing an identifier or the exact name with the query (a crude funnel)."""
        wanted = set(self._flag(argv, "--identifier"))
        name = (self._flag(argv, "--name") or [""])[0]
        return [c for c, chart in self.charts.items()
                if wanted & chart["identifiers"] or (name and name == chart["name"])]

    def add_foreign_lookalike(self, name):
        chart = str(uuid.uuid4())
        self.charts[chart] = {"name": name, "identifiers": set(), "medicines": []}
        return chart

    # -- the verbs -----------------------------------------------------------------------
    def __call__(self, argv):
        self.calls.append(argv)
        verb = argv[0]
        done = self.m.Completed
        if verb == "patient-search":
            return done(0, self._table(self._matching(argv)), "")
        if verb == "patient-register":
            shown = self._matching(argv)
            if shown and "--confirm-new" not in argv:
                return done(1, self._table(shown),
                            f"Error: {len(shown)} candidate(s) already on file\n")
            chart = str(uuid.uuid4())
            self.charts[chart] = {"name": self._flag(argv, "--name")[0],
                                  "identifiers": set(self._flag(argv, "--identifier")),
                                  "medicines": []}
            return done(0, self._table(shown) +
                        f"registered patient {chart}\nregistration event {uuid.uuid4()}\n", "")
        if verb == "medication-assert":
            chart = argv[1]
            written = sum(len(c["medicines"]) for c in self.charts.values())
            if self.fail_medication_after is not None and written >= self.fail_medication_after:
                return done(1, "", "Error: connection closed\n")
            thread = str(uuid.uuid4())
            self.charts[chart]["medicines"].append({
                "thread": thread,
                "term": self._flag(argv, "--term")[0],
                "started_value": self._flag(argv, "--started")[0],
                "coding_code": (self._flag(argv, "--coding-code") or [None])[0],
                "ceased": None,
            })
            return done(0, f"recorded medication for {chart}; thread {thread}; "
                           f"event {uuid.uuid4()}\n", "")
        if verb == "medication-cease":
            for med in self.charts[argv[1]]["medicines"]:
                if med["thread"] == argv[2]:
                    med["ceased"] = self._flag(argv, "--stopped")[0]
            return done(0, "ceased\n", "")
        if verb == "medication-list":
            return done(0, json.dumps({"rows": self.charts[argv[1]]["medicines"]}), "")
        raise AssertionError(f"the importer ran a verb the stand-in does not know: {argv}")

    def writes(self):
        return [c for c in self.calls
                if c[0] in ("patient-register", "medication-assert", "medication-cease")]


# ---------------------------------------------------------------------------------------
# Reading the file
# ---------------------------------------------------------------------------------------


def test_a_file_without_the_synthetic_marker_is_refused(m) -> None:
    """The identifiers are valid in form, so an unmarked file is treated as real data."""
    with tempfile.TemporaryDirectory() as tmp:
        path = write_cohort(tmp, application_id=0)
        try:
            m.read_cohort(path)
        except m.CohortRefused as e:
            assert "marker" in str(e)
        else:
            raise AssertionError("an unmarked file was read")


def test_a_manifest_that_does_not_say_synthetic_is_refused(m) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        path = write_cohort(tmp, provenance="extract")
        try:
            m.read_cohort(path)
        except m.CohortRefused as e:
            assert "synthetic" in str(e)
        else:
            raise AssertionError("a file not marked synthetic was read")


def test_a_file_that_is_not_sqlite_is_refused_not_crashed_on(m) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        path = pathlib.Path(tmp) / "cohort.sqlite"
        path.write_text("MSH|^~\\&|a real HL7 file handed over by mistake\r")
        try:
            m.read_cohort(path)
        except m.CohortRefused:
            pass
        else:
            raise AssertionError("a non-database file was read")


def test_the_cohort_is_read_with_what_must_be_left_behind(m) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        cohort = m.read_cohort(write_cohort(tmp))
    assert [p.mrn for p in cohort.patients] == ["SHD000001", "SHD000002"]
    assert cohort.patients[0].medicines[1].stopped == "2024-01-05"
    assert cohort.left_behind == {
        "sex": 2, "address": 2, "conditions": 3, "pathology results": 7,
        "progress notes": 1, "imaging reports": 1, "specialist letters": 2, "clinicians": 1,
    }
    # Every kind left behind has a stated reason: a count with no explanation is noise.
    assert set(cohort.left_behind) <= set(m.WHY_LEFT_BEHIND)


# ---------------------------------------------------------------------------------------
# Naming the cohort
# ---------------------------------------------------------------------------------------


def test_growing_a_cohort_keeps_its_tag_and_a_new_seed_changes_it(m) -> None:
    """A larger cohort keeps the patients the smaller had, so it must find their charts;
    another seed reuses the same record numbers for other people, so it must not."""
    assert m.cohort_tag(SPEC) == m.cohort_tag({**SPEC, "patients": 5000})
    assert m.cohort_tag(SPEC) != m.cohort_tag({**SPEC, "seed": 2})
    assert m.cohort_tag(SPEC) != m.cohort_tag({**SPEC, "as_of": "2027-01-01"})
    assert m.mrn_system(SPEC).startswith(m.MRN_SYSTEM_PREFIX)


# ---------------------------------------------------------------------------------------
# Command lines
# ---------------------------------------------------------------------------------------


def _patient(m, **over):
    base = dict(mrn="SHD000001", given="Biwei", family="Chipp", birth_date="1955-07-15",
                identifiers=((m.MRN_URI, "SHD000001"), (m.IHI_URI, "8003600006972035"),
                             (m.MEDICARE_URI, "20016275733")),
                medicines=())
    return m.Patient(**{**base, **over})


def test_registration_uses_cairns_spelling_of_each_identifier_system(m) -> None:
    argv = m.register_argv(_patient(m), "mrn-shd-abc", confirm_new=False)
    assert argv == [
        "patient-register", "--name", "Biwei Chipp", "--birth-date", "1955-07-15",
        "--identifier", "mrn-shd-abc=SHD000001",
        "--identifier", "ihi-au=8003600006972035",
        "--identifier", "medicare-au=20016275733",
    ]
    assert "--confirm-new" not in argv


def test_an_identifier_in_an_unknown_system_is_dropped_and_counted_never_guessed(m) -> None:
    """A guessed system name makes two schemes look like one, and a search joins on it."""
    patient = _patient(m, identifiers=((m.MRN_URI, "SHD000001"), ("urn:example:dva", "QX1")))
    args, unspelled = m.cairn_identifiers(patient, "mrn-shd-abc")
    assert args == ["mrn-shd-abc=SHD000001"]
    assert unspelled == 1


def test_a_medicine_is_recorded_as_read_from_another_record(m) -> None:
    plain = m.medication_argv("C", m.Medicine(0, "perindopril", "2019-03-02", None, None))
    assert plain == ["medication-assert", "C", "--term", "perindopril", "--info-source",
                     "external-record", "--started", "2019-03-02", "--started-precision", "day"]
    coded = m.medication_argv(
        "C", m.Medicine(1, "warfarin", "2021-06-10", None, WARFARIN_MOIETY))
    assert coded[-6:] == ["--coding-system", "drugref-moiety", "--coding-code",
                          WARFARIN_MOIETY, "--coding-display", "warfarin"]


def test_a_drug_code_that_is_not_a_moiety_uuid_is_not_sent(m) -> None:
    """The floor refuses anything but a uuid; sending a PBS item there would lose the line."""
    argv = m.medication_argv("C", m.Medicine(0, "warfarin", "2021-06-10", None, "2843P"))
    assert "--coding-code" not in argv


def test_the_real_binarys_output_is_parsed(m) -> None:
    chart, event, thread = (str(uuid.uuid4()) for _ in range(3))
    assert m.parse_registered(
        f"no candidates found\nregistered patient {chart}\nregistration event {event}\n"
    ) == chart
    assert m.parse_recorded_thread(
        f"recorded medication for {chart}; thread {thread}; event {event}\n") == thread
    table = (f"{'patient_id':<36}  {'name':<28}   age  trust\n"
             f"{chart}  {'Biwei Chipp':<28}    71  ok\n"
             f"{thread}  {'↳ linked: B Chipp':<28}     ?  ok\n")
    assert m.parse_candidates(table) == [chart, thread]
    assert m.parse_candidates("no candidates found\n") == []


def test_a_failure_is_reported_by_its_cause_not_its_stack(m) -> None:
    """With RUST_BACKTRACE set the last line of stderr is a stack frame ("34: _start"), which
    is what the first real run reported for every patient."""
    stderr = ("Error: db error\n\nCaused by:\n    ERROR: permission denied for function "
              "cairn_search_candidates\n\nStack backtrace:\n   0: anyhow::error\n  34: _start\n")
    assert m._refusal(stderr) == (
        "db error: permission denied for function cairn_search_candidates")
    assert m._refusal("Error: 2 candidate(s) already on file\n") == (
        "Error: 2 candidate(s) already on file")
    assert m._refusal("") == ""


# ---------------------------------------------------------------------------------------
# The run
# ---------------------------------------------------------------------------------------


def _run(m, node, **kw):
    with tempfile.TemporaryDirectory() as tmp:
        cohort = m.read_cohort(write_cohort(tmp, **kw.pop("cohort", {})))
    return m.import_cohort(cohort, node, m.Options(**kw))


def test_a_fresh_node_receives_every_patient_and_medicine(m) -> None:
    node = FakeNode(m)
    report = _run(m, node)
    assert (report.patients_registered, report.medicines_recorded) == (2, 3)
    assert (report.medicines_coded, report.medicines_ceased) == (1, 1)
    assert not report.incomplete()
    stopped = [med for c in node.charts.values() for med in c["medicines"] if med["ceased"]]
    assert [(s["term"], s["ceased"]) for s in stopped] == [("warfarin", "2024-01-05")]


def test_a_second_run_writes_nothing(m) -> None:
    node = FakeNode(m)
    _run(m, node)
    before = len(node.writes())
    report = _run(m, node)
    assert len(node.writes()) == before, "the second run wrote to the node"
    assert (report.patients_already_present, report.medicines_already_present) == (2, 3)
    assert (report.patients_registered, report.medicines_recorded) == (0, 0)
    assert not report.incomplete()


def test_an_interrupted_run_is_finished_by_running_again(m) -> None:
    node = FakeNode(m)
    node.fail_medication_after = 1  # the first medicine lands, the rest fail
    first = _run(m, node)
    assert first.incomplete() and first.medicines_failed == 2
    node.fail_medication_after = None
    second = _run(m, node)
    assert (second.medicines_recorded, second.medicines_already_present) == (2, 1)
    assert not second.incomplete()
    assert sum(len(c["medicines"]) for c in node.charts.values()) == 3, "a line was duplicated"


def test_a_patient_the_funnel_shows_candidates_for_is_held_not_registered(m) -> None:
    """The funnel exists so a person looks. A script that types --confirm-new for itself
    has deleted it."""
    node = FakeNode(m)
    lookalike = node.add_foreign_lookalike("Biwei Chipp")
    report = _run(m, node)
    assert report.patients_held == 1 and report.patients_registered == 1
    assert report.held[0]["patient"] == "SHD000001"
    assert report.held[0]["charts"] == [lookalike]
    assert not any("--confirm-new" in call for call in node.calls)
    assert report.incomplete()
    assert node.charts[lookalike]["medicines"] == [], "medicines went onto a stranger's chart"


def test_registering_past_candidates_is_an_explicit_choice(m) -> None:
    node = FakeNode(m)
    node.add_foreign_lookalike("Biwei Chipp")
    report = _run(m, node, register_past_candidates=True)
    assert (report.patients_registered, report.patients_held) == (2, 0)
    assert sum("--confirm-new" in call for call in node.calls) == 1


def test_two_charts_under_one_record_number_are_held_for_a_person(m) -> None:
    """Which chart is "the" patient is a linking decision; writing to either is a guess."""
    node = FakeNode(m)
    _run(m, node)
    system = m.mrn_system(SPEC)
    twin = node.add_foreign_lookalike("B. Chipp")
    node.charts[twin]["identifiers"] = {f"{system}=SHD000001"}
    before = len(node.writes())
    report = _run(m, node)
    assert report.patients_held == 1
    assert len(node.writes()) == before


def test_another_seeds_cohort_does_not_reuse_this_cohorts_charts(m) -> None:
    node = FakeNode(m)
    _run(m, node)
    other = [("SHD000001", "Someone", "Else", "1990-01-01", [])]
    report = _run(m, node, cohort={"spec": {**SPEC, "seed": 2}, "people": other})
    assert (report.patients_registered, report.patients_already_present) == (1, 0)


def test_limit_loads_only_the_first_patients(m) -> None:
    node = FakeNode(m)
    report = _run(m, node, limit=1)
    assert (report.patients_registered, report.medicines_recorded) == (1, 2)


def test_the_report_says_what_was_left_and_why(m) -> None:
    node = FakeNode(m)
    report = _run(m, node, cohort={"extra_identifier": ("urn:example:dva", "QX1")})
    text = "\n".join(m.report_lines(report, 2))
    assert "registered 2" in text and "recorded 3 (1 coded)" in text
    assert "7  pathology results: no results stream (#1208)" in text
    assert "2  identifiers in a system Cairn has no spelling for" in text
    assert "per registration" in text


def test_a_newer_file_format_is_read_and_said_to_be_newer(m) -> None:
    """Formats only add tables, so refusing a newer file would break every cohort the day
    the generator grows; saying nothing would hide that this script may be missing a kind."""
    node = FakeNode(m)
    report = _run(m, node, cohort={"database_format": m.KNOWN_DATABASE_FORMAT + 1})
    assert report.newer_format
    assert "newer format" in "\n".join(m.report_lines(report, 2))


# ---------------------------------------------------------------------------------------
# The command line's refusals (no node is reached in any of these)
# ---------------------------------------------------------------------------------------


def test_the_command_refuses_an_unmarked_file_with_exit_2(m) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        path = write_cohort(tmp, application_id=0)
        assert m.main([str(path), "--conn", "host=nowhere"]) == 2


def test_a_dry_run_needs_no_node(m) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        assert m.main([str(write_cohort(tmp)), "--dry-run"]) == 0


def test_the_command_refuses_a_node_holding_anyone_elses_charts(m) -> None:
    """Until Cairn can mark a chart as invented (#1207) this guard is all there is."""
    real_count = m.foreign_chart_count
    m.foreign_chart_count = lambda conn: 4
    try:
        with tempfile.TemporaryDirectory() as tmp:
            assert m.main([str(write_cohort(tmp)), "--conn", "host=x"]) == 2
    finally:
        m.foreign_chart_count = real_count


def test_the_guard_counts_only_charts_without_a_cohort_record_number(m) -> None:
    sql = m.FOREIGN_CHARTS_SQL
    assert f"LIKE '{m.MRN_SYSTEM_PREFIX}%'" in sql and "NOT EXISTS" in sql


def main() -> int:
    m = load_script()
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    # A runner that collects tests by NAME PREFIX must say how many it expected to find:
    # rename the prefix and an empty list prints "0 passed" and exits 0.
    if len(tests) < 25:
        raise AssertionError(
            f"collected only {len(tests)} tests; this file has more than that. The collector "
            "matches names starting `test_`, so a rename can silently empty it."
        )
    for t in tests:
        t(m)
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
