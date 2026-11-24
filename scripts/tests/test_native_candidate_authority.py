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

    def picker_receipt(self):
        release = json.loads((ROOT / 'artifacts/build-info.json').read_text())
        return dict(
            status='finished', pickerPreflightPassed=True, qaQuitNormally=True,
            temporaryDirectoryRemoved=True, sourceUnchanged=True, normalPreserved=True,
            acceptedRelease=dict(zipSha256=release['sha256'],
                                 executableSha256=release['executableSha256']),
            events=[
                dict(stage='nativeForegroundObserved', pid=42, applicationAsn='ASN:qa:', frontAsn='ASN:qa:'),
                *[dict(stage='nativePickerPreflightCheckpoint', step=step, pid=42)
                  for step in ('empty', 'imported', 'backed-up', 'file-reviewing')],
            ])

    def test_candidate_requires_complete_preflight_for_the_current_release_before_registration(self):
        with tempfile.TemporaryDirectory(prefix='travel-gallery-picker-gate-') as owned:
            work = Path(owned)
            preflight = work / 'preflight.json'
            descriptor, name = tempfile.mkstemp(prefix='catalog-picker-', suffix='.json', dir=ROOT / 'docs')
            os.close(descriptor)
            registration = Path(name)
            registration.unlink()
            self.addCleanup(registration.unlink, missing_ok=True)
            with (patch.object(candidate, 'PICKER_PREFLIGHT', preflight),
                  patch.object(candidate.native_gui, 'require_unlocked_console')):
                with self.assertRaisesRegex(RuntimeError, 'picker preflight before'):
                    candidate.register_standing(registration, 'catalog-snapshot-deadline-test')
                self.assertFalse(registration.exists())
                good = self.picker_receipt()
                for key, value in [('status', 'failed'), ('qaQuitNormally', False),
                                   ('temporaryDirectoryRemoved', False), ('normalPreserved', False),
                                   ('forcedCleanupPids', [42])]:
                    data = dict(good, **{key: value})
                    preflight.write_text(json.dumps(data))
                    with self.subTest(key=key), self.assertRaises(RuntimeError):
                        candidate.require_picker_preflight()
                for data in [
                    dict(good, events=good['events'][:-1]),
                    dict(good, acceptedRelease=dict(good['acceptedRelease'], zipSha256='old-release')),
                    dict(good, events=[dict(good['events'][0], frontAsn='ASN:other:'), *good['events'][1:]]),
                    dict(good, events=[*good['events'][:-1], dict(good['events'][-1], pid=43)]),
                ]:
                    preflight.write_text(json.dumps(data))
                    with self.assertRaises(RuntimeError):
                        candidate.require_picker_preflight()
                preflight.write_text(json.dumps(good))
                candidate.register_standing(registration, 'catalog-snapshot-deadline-test')
                expected = candidate.require_picker_preflight()
                self.assertEqual(json.loads(registration.read_text())['pickerPreflightSha256'], expected)
                # Even another valid receipt must not silently replace the one bound to the batch.
                preflight.write_text(json.dumps(dict(good, note='later successful preflight')))
                before = registration.read_bytes()
                with self.assertRaisesRegex(RuntimeError, 'changed after registration'):
                    with candidate.claim_approval(registration, 'catalog-snapshot-deadline-test'):
                        self.fail('A different preflight was silently accepted')
                self.assertEqual(registration.read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
