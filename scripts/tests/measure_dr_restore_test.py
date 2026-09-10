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
text invented to make a parser pass.

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


INIT_OUTPUT = """
=== RECOVERY CODE — shown ONCE. Write it down; store it OFF-SITE. ===
    R9RQ0-3XZAB-ZCNN8-51WGR-QSXP2-H13MA-SV
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
        self.assertEqual(
            rig.parse_recovery_code(INIT_OUTPUT),
            "R9RQ0-3XZAB-ZCNN8-51WGR-QSXP2-H13MA-SV",
        )

    def test_a_missing_code_raises_rather_than_returning_none(self) -> None:
        """Silence here would strand the rig at an unanswerable prompt minutes later.

        Failing at the point the code should have appeared names the real cause;
        a `None` returned now surfaces as a timeout during the restore, which
        reads like a slow restore and would be recorded as one.
        """
        with self.assertRaises(rig.RigError):
            rig.parse_recovery_code("provisioned node 1220ab\nfingerprint 408D\n")

    def test_it_does_not_mistake_the_banner_for_the_code(self) -> None:
        # The banner lines are all-caps with dashes too; only the indented
        # group-of-five-characters shape is the code.
        code = rig.parse_recovery_code(INIT_OUTPUT)
        self.assertNotIn("RECOVERY", code)
        self.assertNotIn("=", code)


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

    def test_the_budget_is_ten_minutes_and_is_not_a_parameter(self) -> None:
        """The budget is #512's, not the rig's, so it is a constant here.

        Making it a flag is how a measurement quietly gets graded against a
        budget somebody widened to make a run pass.
        """
        self.assertEqual(rig.BUDGET_SECONDS, 600)


if __name__ == "__main__":
    unittest.main(verbosity=2)
