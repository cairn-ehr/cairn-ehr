"""propose()/sweep() must pin the config they ACTUALLY used, not the defaults (issue #100).

The persisted matcher_version is the ADR-0011/0029 contamination-recall handle, so every
entry point has to thread its own thresholds/weights/comparator config into the payload's
pin — a caller running custom thresholds whose proposals still pin the defaults would make
a bad rollout indistinguishable in the proposal table.

Pure — no database, no psycopg. propose() imports `cairn_matcher.pipeline.db` lazily
inside the call, so these tests plant a stub module at that seam (both in sys.modules and
as the package attribute, covering the run orders where the real module was / was not
already imported by a DB-gated test earlier in the session). The stub serves two synthetic
records with an EXACT dob (weight 6.0 at verified provenance), which lands in REVIEW under
the default Thresholds(review=3, auto=8) — a persisted proposal whose payload we capture.
"""

import sys
import types

from cairn_matcher.orchestrator import DEFAULT_CONFIG
from cairn_matcher.pipeline.banding import Band, Thresholds, matcher_version
from cairn_matcher.records import CandidateRecord, DateValue, FieldValue

A = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
B = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"


class _StubConn:
    """The only two connection methods propose() touches. No transaction semantics."""

    def commit(self) -> None:
        pass

    def rollback(self) -> None:
        pass


def _install_db_stub(monkeypatch) -> dict:
    """Plant a stand-in cairn_matcher.pipeline.db and return the capture dict.

    Every function propose() calls on the module is stubbed; upsert_proposal records the
    payload it was handed so a test can assert on the persisted matcher_version.
    """
    captured: dict = {}
    stub = types.ModuleType("cairn_matcher.pipeline.db")

    def _record(conn, pid):
        # Same verified DOB on both sides -> dob EXACT at full provenance factor.
        return CandidateRecord(dob=FieldValue(DateValue(year=1980, month=1, day=2), 70))

    stub.load_candidate = _record
    stub.match_veto = lambda conn, a, b: []
    stub.load_aliases = lambda conn, pid: frozenset()
    stub.load_trust_for = lambda conn, ids: {}
    stub.retract_pending_proposal = lambda conn, low, high: False
    stub.upsert_proposal = lambda conn, low, high, payload: captured.update(payload=payload)

    # Cover both lazy-import resolutions of `from cairn_matcher.pipeline import db`:
    # the sys.modules lookup (nothing imported the real module yet — the pure-pytest
    # environment, where psycopg is absent) and the package-attribute lookup (a DB-gated
    # test already imported it earlier in this pytest session).
    import cairn_matcher.pipeline as pipeline_pkg

    monkeypatch.setitem(sys.modules, "cairn_matcher.pipeline.db", stub)
    monkeypatch.setattr(pipeline_pkg, "db", stub, raising=False)
    return captured


def test_propose_pins_the_thresholds_it_used(monkeypatch):
    from cairn_matcher.pipeline.runner import propose

    captured = _install_db_stub(monkeypatch)
    custom = Thresholds(review=1.0, auto=10.0)

    assert propose(_StubConn(), A, B, thresholds=custom) is Band.REVIEW
    payload = captured["payload"]
    assert payload.matcher_version == matcher_version(thresholds=custom)
    assert payload.matcher_version != matcher_version()


def test_propose_threads_a_custom_comparator_config_end_to_end(monkeypatch):
    from cairn_matcher.pipeline.runner import propose

    captured = _install_db_stub(monkeypatch)
    dob_only = (DEFAULT_CONFIG[0],)  # just the dob FieldSpec

    assert propose(_StubConn(), A, B, config=dob_only) is Band.REVIEW
    payload = captured["payload"]
    # The config reached field_comparisons (only dob evidence exists) ...
    assert [e["field"] for e in payload.evidence if "field" in e] == ["dob"]
    # ... AND the pin, so the persisted row records the wiring that scored it.
    assert payload.matcher_version == matcher_version(config=dob_only)
    assert payload.matcher_version != matcher_version()


C = "cccccccc-cccc-cccc-cccc-cccccccccccc"


def test_sweep_threads_its_config_into_both_propose_paths(monkeypatch):
    # The batch driver calls propose() twice — the main candidate loop and the #210
    # reconciliation pass for orphaned pending pairs. BOTH must carry the sweep's own
    # comparator config, or a custom-config sweep persists proposals pinned to the
    # defaults (the same recall-key gap, on the batch path). sweep()'s own db calls are
    # stubbed; propose is replaced with a capturing fake (the test_sweep flaky idiom) —
    # the pin-correctness of propose itself is proven by the tests above.
    from cairn_matcher.pipeline import sweep as sweep_mod

    _install_db_stub(monkeypatch)
    db_stub = sys.modules["cairn_matcher.pipeline.db"]
    db_stub.generate_candidate_pairs = (
        lambda conn, max_block_size: ([(A, B)], [])
    )
    db_stub.load_aliases_for = lambda conn, ids: {}
    db_stub.pending_proposal_pairs = lambda conn: [(A, C)]  # an orphan for reconciliation

    calls: list[dict] = []

    def _capturing_propose(conn, a, b, **kwargs):
        calls.append(kwargs)
        return Band.REVIEW

    monkeypatch.setattr(sweep_mod, "propose", _capturing_propose)
    dob_only = (DEFAULT_CONFIG[0],)

    result = sweep_mod.sweep(_StubConn(), config=dob_only)
    assert result.generated == 1 and result.reconciled == 1
    assert len(calls) == 2  # main-loop pair + reconciliation orphan
    assert all(kw["config"] == dob_only for kw in calls)
