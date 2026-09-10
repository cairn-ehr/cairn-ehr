#!/usr/bin/env python3
"""Tests for scripts/measure_dr_restore.py — the #512 paper-parity measurement rig.

The rig drives real binaries against a real database, so almost none of it can be
tested here. What CAN be tested is the part that decides **what number gets
written down**: the parsers that read a recovery code and a clinical summary out
of operator-facing text.

That is the half worth guarding. A rig that mis-parses does not crash — it
reports a plausible wrong figure into a dated results file that outlives the
session, and house rule 7 says a figure outside its budget is a finding. A
finding derived from a mis-read line is worse than no finding.

Every fixture below is **verbatim output from a real run** on 2026-09-10, not
text invented to make a parser pass — with one deliberate exception: the recovery
code itself is derived at runtime rather than committed, because a real one is
cryptographic material (house rule 6). Its shape is the shipped shape, which is
all the parser looks at.

Standard library only, no pytest — run it directly, the way
`scripts/tests/check_closing_keywords_test.py` is run:

    python3 scripts/tests/measure_dr_restore_test.py
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

# The rig lives one directory up. Import it by path so this file runs from
# anywhere without the repo needing to be an installed package.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import measure_dr_restore as rig  # noqa: E402  (path set above)


#: Crockford base32, the alphabet `generate_recovery_code` draws from — no I, L,
#: O or U. Kept here so the sample below is built the way a real code is shaped.
_B32 = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

#: A recovery code of exactly the shipped shape — six groups of five plus a final
#: two, 38 characters — **derived at runtime, never a literal.**
#:
#: House rule 6: a real code is 160 bits of off-node escrow for a node key, and
#: writing one into a repository is writing down cryptographic material even when
#: the node it belonged to was a throwaway in /tmp. The banner and the surrounding
#: lines below ARE verbatim from a real run, which is what makes them a useful
#: fixture; only the secret itself is synthetic, and the parser cannot tell the
#: difference because it matches on shape.
SAMPLE_RECOVERY_CODE = "-".join(
    ["".join(_B32[(group * 7 + position) % len(_B32)] for position in range(5))
     for group in range(6)]
    + ["".join(_B32[position] for position in range(2))]
)

INIT_OUTPUT = f"""
=== RECOVERY CODE — shown ONCE. Write it down; store it OFF-SITE. ===
    {SAMPLE_RECOVERY_CODE}
=== This is the only off-node way to recover this node's signing key. ===
=== Lose BOTH this code and the passphrase and the node is permanently ===
=== lost — recoverable only by re-provisioning a new identity. ===

local-state escrow established at /tmp/cairn-measure-node.key.lsk
unwrap key established at /tmp/cairn-measure-node.key.unwrap
provisioned node 122017426cb2abbabf78c6013ab3666096d1791fa12e99a2555dc2a9348d5010e6e6
fingerprint 408D-0788-795E-381D-E1C0
"""

GOOD_RESTORE = """
local-state restored from /tmp/cairn-dr-medium/medium.cairnb.localstate
custody inherited: unwrap key installed and registered (340 episode DEK(s) carried)
actor registry restored: 2 of 2 carried row(s) inserted
restored 1 event(s) from /tmp/cairn-dr-medium/medium.cairnb
clinical records: 403 applied, 0 already present, 0 refused (of 403 on the medium)
restored identity 'measure-rig' (127.0.0.1:0)
"""

# The zero-patients outcome, produced for real by piping the recovery code into a
# non-tty. This is the shape the rig must never score as a success.
ZERO_PATIENTS_RESTORE = """
Local-state export found. Enter the OLD node's recovery code to unseal it:
ERROR: the local-state export could not be applied: Device not configured (os error 6)
WARNING: this node has NO actor registry, so the apply door will refuse every clinical record.
restored 1 event(s) from /tmp/cairn-dr-medium/medium.cairnb
clinical records: 0 applied, 0 already present, 0 refused (of 403 on the medium)
"""


class ReadsTheRecoveryCode(unittest.TestCase):
    def test_it_finds_the_code_printed_once(self) -> None:
        self.assertEqual(rig.parse_recovery_code(INIT_OUTPUT), SAMPLE_RECOVERY_CODE)

    def test_the_shipped_shape_is_38_characters(self) -> None:
        """Six groups of five plus a final two, dash-joined.

        `generate_recovery_code` base32-encodes 160 bits, which is 32 characters,
        and chunks them in fives. The published results file said 37; it is 38, and
        that is the kind of number a reader checks by counting.
        """
        self.assertEqual(len(SAMPLE_RECOVERY_CODE), 38)
        self.assertEqual([len(g) for g in SAMPLE_RECOVERY_CODE.split("-")],
                         [5, 5, 5, 5, 5, 5, 2])

    def test_a_missing_code_raises_rather_than_returning_none(self) -> None:
        """Silence here would strand the rig at an unanswerable prompt minutes later.

        Failing at the point the code should have appeared names the real cause;
        a `None` returned now surfaces as a timeout during the restore, which
        reads like a slow restore and would be recorded as one.
        """
        with self.assertRaises(rig.RigError):
            rig.parse_recovery_code("provisioned node 1220ab\nfingerprint 408D\n")

    def test_the_banner_alone_yields_no_code(self) -> None:
        """Banner text must not be mistaken for a code — tested by removing the code.

        The previous version of this test asserted that the returned code contained
        neither "RECOVERY" nor "=", which the pattern makes structurally impossible:
        no group is wider than five characters, so an eight-character word can never
        appear in a match. It could not fail for any input that parsed at all.

        Feeding the banner WITHOUT a code is the real question, and the answer must
        be a raise rather than a banner fragment.
        """
        banner_only = INIT_OUTPUT.replace(SAMPLE_RECOVERY_CODE, "")
        with self.assertRaises(rig.RigError):
            rig.parse_recovery_code(banner_only)

    def test_the_fingerprint_is_not_mistaken_for_the_code(self) -> None:
        """The other dashed, upper-case token `init` prints.

        Its groups are four characters wide; a code's are five, with at least six
        of them. Shape is what separates them, so this pins the shape.
        """
        with self.assertRaises(rig.RigError):
            rig.parse_recovery_code("fingerprint 408D-0788-795E-381D-E1C0\n")


class ReadsTheClinicalSummary(unittest.TestCase):
    def test_it_reads_all_four_numbers(self) -> None:
        summary = rig.parse_clinical_summary(GOOD_RESTORE)
        self.assertEqual(summary.applied, 403)
        self.assertEqual(summary.already_present, 0)
        self.assertEqual(summary.refused, 0)
        self.assertEqual(summary.on_medium, 403)

    def test_a_complete_restore_is_recognised(self) -> None:
        self.assertTrue(rig.parse_clinical_summary(GOOD_RESTORE).is_complete)

    def test_the_zero_patients_outcome_is_not_a_success(self) -> None:
        """#500's own signature: a clean-looking summary over an empty restore.

        The rig must refuse to record a timing for it. A restore that applied
        nothing is fast, and a fast wrong number is exactly what would be quoted
        against the budget later.
        """
        summary = rig.parse_clinical_summary(ZERO_PATIENTS_RESTORE)
        self.assertEqual(summary.applied, 0)
        self.assertEqual(summary.on_medium, 403)
        self.assertFalse(summary.is_complete)

    def test_a_partial_restore_is_not_complete(self) -> None:
        text = "clinical records: 400 applied, 0 already present, 3 refused (of 403 on the medium)"
        summary = rig.parse_clinical_summary(text)
        self.assertEqual(summary.refused, 3)
        self.assertFalse(summary.is_complete)

    def test_already_present_records_count_toward_completeness(self) -> None:
        """A re-restore into a partially-populated database is still complete.

        `already present` is the door reporting an idempotent no-op, not a loss.
        Scoring it as a shortfall would make a resumed restore look like a
        failed one.
        """
        text = "clinical records: 3 applied, 400 already present, 0 refused (of 403 on the medium)"
        self.assertTrue(rig.parse_clinical_summary(text).is_complete)

    def test_an_empty_medium_is_not_a_complete_restore(self) -> None:
        """Nothing on the medium, nothing applied: vacuously "complete", actually empty.

        Every other clause is satisfied trivially — nothing refused, nothing
        missing — so without an explicit `on_medium > 0` the rig would record a
        one-second timing as a PASS. `restore` happens not to print the summary
        line at all today when it carried no clinical records, which makes this
        unreachable through the CLI; that is a detail of a Rust file this suite
        cannot see change, so the invariant is pinned here.
        """
        self.assertFalse(rig.ClinicalSummary(0, 0, 0, 0).is_complete)

    def test_a_refusal_alone_defeats_completeness(self) -> None:
        """The `refused == 0` clause, tested where nothing else fires.

        The existing partial-restore case trips the count clause AND the refusal
        clause at once, so deleting `refused == 0` from the predicate left it
        green. Here the counts add up perfectly and only the refusal is wrong.
        """
        self.assertFalse(rig.ClinicalSummary(403, 0, 2, 403).is_complete)

    def test_a_missing_summary_raises(self) -> None:
        with self.assertRaises(rig.RigError):
            rig.parse_clinical_summary("restored identity 'measure-rig'\n")


class FormatsTheResultsTable(unittest.TestCase):
    def test_the_table_reports_the_count_the_restore_itself_found(self) -> None:
        """Not a count derived from the seed parameters.

        The first draft carried both and they disagreed — the seeder counts the
        events it authored, the restore counts what it found on the medium, and
        the medium also holds the warm-up registration. The results file must
        quote the second, so the first is not kept at all.
        """
        rows = [
            rig.Measurement(
                seed_s=0.4, backup_s=0.2, restore_s=1.1, applied=403, on_medium=403
            ),
            rig.Measurement(
                seed_s=110.0, backup_s=9.0, restore_s=240.0,
                applied=100_003, on_medium=100_003,
            ),
        ]
        table = rig.format_results_table(rows)
        self.assertIn("403", table)
        self.assertIn("100003", table.replace(",", ""))
        self.assertIn("240.0", table)

    def test_the_budget_verdict_is_stated_per_row(self) -> None:
        """House rule 7: a figure outside its budget is the finding.

        So the verdict belongs in the table beside the number, not in prose a
        reader has to reconcile against it.
        """
        under = rig.Measurement(
            seed_s=1.0, backup_s=1.0, restore_s=5.0, applied=1000, on_medium=1000
        )
        over = rig.Measurement(
            seed_s=1.0, backup_s=1.0, restore_s=601.0, applied=1000, on_medium=1000
        )
        self.assertTrue(under.within_budget)
        self.assertFalse(over.within_budget)
        self.assertIn("PASS", rig.format_results_table([under]))
        self.assertIn("FAIL", rig.format_results_table([over]))

    def test_each_measured_number_lands_in_its_own_column(self) -> None:
        """The mapping from `Measurement` field to published column, pinned exactly.

        This is the only place a measured float becomes a figure somebody quotes,
        and it was previously "tested" by three substring assertions against a
        whole rendered table — which cannot tell WHICH column a number is in.
        Transposing the Backup and Restore cells in the f-string left the entire
        suite green while writing the backup time into the column headed Restore,
        i.e. a wrong number in the one column the whole measurement exists to
        produce and grade against the budget.

        Every value here is distinct on purpose. Two equal values are two values a
        transposition cannot be seen through, and the old fixtures set `applied`
        equal to `on_medium` in every row.
        """
        table = rig.format_results_table([
            rig.Measurement(
                seed_s=1.0, backup_s=2.0, restore_s=3.0, applied=4, on_medium=5
            )
        ]).splitlines()
        self.assertEqual(
            table[0],
            "| Events on medium | Seed (s) | Backup (s) | **Restore (s)** | "
            "Applied | \u2264 10 min |",
        )
        self.assertEqual(table[2], "| 5 | 1.0 | 2.0 | **3.0** | 4 | PASS |")

    def test_the_defaults_are_the_published_curve(self) -> None:
        """A bare invocation must reproduce the recorded figures.

        The default drifted to `100,1000,3000,6000` once, so a bare run would have
        produced points that do not line up with the dated results file. It was
        caught by eye in a follow-up commit. This is the guard that would have
        caught it instead.
        """
        runbook = (
            Path(__file__).resolve().parents[2]
            / "crates" / "cairn-node" / "results" / "RUNBOOK.md"
        ).read_text()
        self.assertIn(f"--sizes {rig.DEFAULT_SIZES}", runbook)
        self.assertEqual(rig.DEFAULT_MEDS_PER_PATIENT, 17)
        self.assertIn(f"default `{rig.DEFAULT_MEDS_PER_PATIENT}`", runbook)

    def test_the_budget_boundary_is_inclusive(self) -> None:
        """Exactly at the budget is a PASS; a tenth of a second over is not.

        This one comparison is what turns a measurement into a house-rule-7
        finding, and it was only ever exercised far from its boundary.
        """
        at = rig.Measurement(
            seed_s=1.0, backup_s=1.0, restore_s=float(rig.BUDGET_SECONDS),
            applied=1, on_medium=1,
        )
        over = rig.Measurement(
            seed_s=1.0, backup_s=1.0, restore_s=rig.BUDGET_SECONDS + 0.1,
            applied=1, on_medium=1,
        )
        self.assertTrue(at.within_budget)
        self.assertFalse(over.within_budget)

    def test_the_budget_is_ten_minutes_and_is_not_a_parameter(self) -> None:
        """The budget is #512's, not the rig's, so it is a constant here.

        Making it a flag is how a measurement quietly gets graded against a
        budget somebody widened to make a run pass.
        """
        self.assertEqual(rig.BUDGET_SECONDS, 600)


if __name__ == "__main__":
    unittest.main(verbosity=2)
