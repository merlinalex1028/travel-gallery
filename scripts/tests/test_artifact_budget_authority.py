"""Exercise the real budget CLI under legacy and standing resource authorization."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BudgetAuthority(unittest.TestCase):
    def run_budget(self, **changes):
        approval = dict(
            status='active', limits=dict(files=25000, logicalBytes=4 * 1024**3, sourceFiles=500),
            authorizationBasis='AGENTS.md:standing-resource-2026-11-23',
            authorizationFileSha256=hashlib.sha256((ROOT / 'AGENTS.md').read_bytes()).hexdigest(),
            cleanupPolicy='end-of-batch')
        approval.update(changes)
        with tempfile.NamedTemporaryFile(mode='w', dir=ROOT / 'docs', suffix='.json') as handle:
            json.dump(approval, handle)
            handle.flush()
            return subprocess.run(
                [sys.executable, '-B', str(ROOT / 'scripts/check-test-artifact-budget.py'),
                 '--approval-file', handle.name, '--planned-files', '25000',
                 '--planned-bytes', str(4 * 1024**3), '--planned-source-files', '500'],
                capture_output=True, text=True)

    def test_current_standing_authority_needs_no_invented_user_answer(self):
        result = self.run_budget()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)['passed'])

    def test_legacy_explicit_batch_remains_valid(self):
        result = self.run_budget(userAnswer='允许这一轮，结束即清理', authorizationBasis=None)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_stale_inactive_missing_cleanup_or_unbounded_records_fail(self):
        for change in [
            dict(status='closed'), dict(status='proposed'), dict(authorizationBasis=None),
            dict(authorizationFileSha256='old-policy'), dict(cleanupPolicy=None),
            dict(limits=dict(files=25000, logicalBytes=float('inf'), sourceFiles=500)),
            dict(limits=dict(files=-1, logicalBytes=4 * 1024**3, sourceFiles=500)),
            dict(limits=dict(files=25000, logicalBytes=4 * 1024**3)),
        ]:
            with self.subTest(change=change):
                self.assertNotEqual(self.run_budget(**change).returncode, 0)


if __name__ == '__main__':
    unittest.main()
