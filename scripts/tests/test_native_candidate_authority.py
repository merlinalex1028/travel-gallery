"""Exercise candidate registration and single-use claims without building an App."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('candidate', ROOT / 'scripts/native-library-query-build-batch.py')
candidate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(candidate)


class CandidateAuthority(unittest.TestCase):
    def new_registration(self):
        descriptor, name = tempfile.mkstemp(prefix='catalog-claim-', suffix='.json', dir=ROOT / 'docs')
        os.close(descriptor)
        path = Path(name)
        path.unlink()
        self.addCleanup(path.unlink, missing_ok=True)
        with patch.object(candidate.native_gui, 'require_unlocked_console'):
            candidate.register_standing(path, 'catalog-test')
        return path

    def test_standing_claim_uses_actual_budget_gate_and_keeps_the_run_lock(self):
        path = self.new_registration()
        with patch.object(candidate.native_gui, 'require_unlocked_console'):
            with candidate.claim_approval(path, 'catalog-test') as approval:
                self.assertIn('claimedAt', approval)
                self.assertNotIn('userAnswer', approval)
                self.assertEqual(approval['cleanupPolicy'], 'end-of-batch')
                with self.assertRaises(BlockingIOError):
                    with candidate.claim_approval(path, 'catalog-test'):
                        self.fail('A concurrent claim was accepted')
            claimed = path.read_bytes()
            with self.assertRaises(RuntimeError):
                with candidate.claim_approval(path, 'catalog-test'):
                    self.fail('A consumed claim was accepted')
            self.assertEqual(path.read_bytes(), claimed)

    def test_legacy_explicit_batch_remains_accepted(self):
        path = self.new_registration()
        data = json.loads(path.read_text())
        for key in ('authorizationBasis', 'authorizationFileSha256', 'cleanupPolicy'):
            data.pop(key)
        data['userAnswer'] = '允许这一轮，结束即清理'
        path.write_text(json.dumps(data))
        with patch.object(candidate.native_gui, 'require_unlocked_console'):
            with candidate.claim_approval(path, 'catalog-test') as approval:
                self.assertIn('claimedAt', approval)

    def test_locked_console_and_stale_policy_do_not_consume_the_claim(self):
        path = self.new_registration()
        before = path.read_bytes()
        with patch.object(candidate.native_gui, 'require_unlocked_console', side_effect=RuntimeError('locked')):
            with self.assertRaisesRegex(RuntimeError, 'locked'):
                with candidate.claim_approval(path, 'catalog-test'):
                    self.fail('A locked-console claim was accepted')
        self.assertEqual(path.read_bytes(), before)
        data = json.loads(before)
        data['authorizationFileSha256'] = 'stale'
        path.write_text(json.dumps(data))
        before = path.read_bytes()
        with self.assertRaises(subprocess.CalledProcessError):
            with candidate.claim_approval(path, 'catalog-test'):
                self.fail('An outdated policy was accepted')
        self.assertEqual(path.read_bytes(), before)

    def test_closed_registration_cannot_be_overwritten_or_claimed(self):
        path = self.new_registration()
        data = json.loads(path.read_text())
        data['status'] = 'closed'
        path.write_text(json.dumps(data))
        before = path.read_bytes()
        with patch.object(candidate.native_gui, 'require_unlocked_console'):
            with self.assertRaises(FileExistsError):
                candidate.register_standing(path, 'catalog-test')
            with self.assertRaises(RuntimeError):
                with candidate.claim_approval(path, 'catalog-test'):
                    self.fail('A closed batch was accepted')
        self.assertEqual(path.read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
