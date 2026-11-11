"""Check the combined QA driver with a bounded fixture and an in-memory catalog."""
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
spec = importlib.util.spec_from_file_location('batch', ROOT / 'scripts/combined-native-batch.py')
batch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(batch)


class CombinedBatchChecks(unittest.TestCase):
    def test_budget_requires_a_fresh_matching_approval(self):
        record = json.loads(batch.APPROVAL.read_text())
        record.update(status='active', userAnswer='允许这一轮，结束即清理')
        self.assertTrue(batch.authorized(record))
        for update in ({'status': 'proposed'}, {'claimedAt': 'already used'}, {'userAnswer': 'other batch'},
                       {'batch': 'other batch'}, {'limits': dict(batch.LIMITS, files=40001)}):
            self.assertFalse(batch.authorized(dict(record, **update)), update)

    def test_replacement_and_database_verifier(self):
        with tempfile.TemporaryDirectory(prefix='travel-gallery-combined-driver-') as temporary:
            root = Path(temporary).resolve()
            source = root / 'source'
            source.mkdir()
            batch.write_sources(source)
            before = batch.inventory(root)
            self.assertEqual(len(before), 125)
            self.assertEqual(sum(stat[1] for stat in before.values()), 25541)
            self.assertEqual(len({before[f'sort-{i:03}.tiff'][0] for i in range(121)}), 1)
            held = source.with_name('held-out')
            source.rename(held)
            self.assertEqual(batch.inventory(root), before)
            held.rename(source)
            with closing(sqlite3.connect(':memory:')) as conn:
                self._check_catalog(conn, source, before)
            subprocess.run([sys.executable, str(ROOT / 'scripts/check-test-artifact-budget.py'),
                            '--extra-root', temporary], check=True)

    def test_profile_probe_checks_the_process_and_catalog_lock(self):
        with tempfile.TemporaryDirectory(prefix='travel-gallery-combined-profile-') as temporary:
            root = Path(temporary).resolve()
            app = root / 'Owned Process.app'
            executable = app / 'Contents/MacOS/travel-gallery'
            executable.parent.mkdir(parents=True)
            shutil.copy('/bin/sleep', executable)
            data = root / 'isolated'
            data.mkdir()
            with closing(sqlite3.connect(data / 'catalog.sqlite')) as setup:
                setup.execute('CREATE TABLE marker(value)')
                setup.commit()
            with patch.object(batch.b, 'APP_PATH', app), patch.object(batch.b, 'DATA', data):
                with (data / 'catalog.lock').open('wb') as lock:
                    child = subprocess.Popen([str(executable), '10'], pass_fds=(lock.fileno(),))
                    try:
                        with self.assertRaisesRegex(RuntimeError, 'not the batch-owned QA executable'):
                            batch.qa_database(os.getpid())
                        with closing(batch.qa_database(child.pid)) as conn:
                            self.assertEqual(conn.execute('SELECT count(*) FROM marker').fetchone(), (0,))
                            with self.assertRaisesRegex(sqlite3.OperationalError, 'readonly'):
                                conn.execute('INSERT INTO marker VALUES(1)')
                        with patch.object(batch.b, 'DATA', root / 'other'):
                            with self.assertRaisesRegex(RuntimeError, 'expected isolated catalog lock'):
                                batch.qa_database(child.pid)
                    finally:
                        child.terminate()
                        child.wait(timeout=3)
            subprocess.run([sys.executable, str(ROOT / 'scripts/check-test-artifact-budget.py'),
                            '--extra-root', temporary], check=True)

    def _check_catalog(self, conn, source, before):
        migrations = ROOT / 'src-tauri/migrations'
        for name in ('001_baseline.sql', '002_interactions.sql'):
            conn.executescript((migrations / name).read_text())
        conn.execute('INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES(?,?,?,?,?)',
                     ('owned', 'Fixture', str(source), 1, 1))
        conn.execute("INSERT INTO similar_groups VALUES('old','exact','SHA-256','1',NULL,0)")
        for i, (name, (_, size, stamp, _)) in enumerate(sorted(before.items())):
            aid = f'a{i:03}'
            conn.execute("""INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,
                         byte_size,mtime_ms,indexed_at,updated_at) VALUES(?,'owned',?,?,'tiff','image',?,?,0,0)""",
                         (aid, name, name, size, stamp // 10**6))
            if i < 121:
                conn.execute("INSERT INTO similar_group_items VALUES('old',?,NULL,0)", (aid,))
        with self.assertRaisesRegex(RuntimeError, 'Disable watching'):
            batch.replace_late_members(conn, source)
        self.assertEqual(batch.inventory(source.parent), before)
        conn.execute("UPDATE source_roots SET is_enabled=0")
        gid, changed = batch.replace_late_members(conn, source)
        self.assertEqual(changed, {'a119': 'sort-119.tiff', 'a120': 'sort-120.tiff'})
        after = batch.inventory(source.parent)
        self.assertEqual({name for name in before if before[name] != after[name]}, set(changed.values()))
        self.assertEqual({after[name][1] for name in changed.values()}, {143})

        # Synthetic expected rows exercise the driver's independent assertions.
        conn.execute('DELETE FROM similar_groups')
        for group in ('remainder', 'pair'):
            conn.execute("INSERT INTO similar_groups VALUES(?,'exact','SHA-256','1',NULL,0)", (group,))
        for i in range(121):
            aid, name = f'a{i:03}', f'sort-{i:03}.tiff'
            digest, size, stamp, _ = after[name]
            conn.execute('UPDATE media_assets SET sha256=?,byte_size=?,mtime_ms=?,sha256_byte_size=?,sha256_mtime_ms=? WHERE id=?',
                         (digest, size, stamp // 10**6, size, stamp // 10**6, aid))
            conn.execute('INSERT INTO similar_group_items VALUES(?,?,NULL,0)', ('pair' if i >= 119 else 'remainder', aid))
        checkpoint = json.dumps(dict(operation='recomputeExact', groupIds=[gid]))
        conn.execute("INSERT INTO jobs(id,type,status,checkpoint_json,created_at,updated_at) VALUES('auto','HASH_EXACT','completed',?,0,0)",
                     (checkpoint,))
        self.assertEqual(batch.verify_recompute(conn, source, gid, changed), 'auto')
        conn.execute("UPDATE media_assets SET sha256_byte_size=99 WHERE id='a120'")
        with self.assertRaisesRegex(RuntimeError, 'hash version'):
            batch.verify_recompute(conn, source, gid, changed)
        conn.execute("UPDATE media_assets SET sha256_byte_size=byte_size")
        conn.execute("UPDATE similar_group_items SET manually_excluded=1 WHERE asset_id='a119'")
        with self.assertRaisesRegex(RuntimeError, 'Group decisions'):
            batch.verify_recompute(conn, source, gid, changed)
        conn.execute("INSERT INTO jobs(id,type,status,checkpoint_json,created_at,updated_at) VALUES('extra','HASH_EXACT','queued',?,0,0)",
                     (checkpoint,))
        with self.assertRaisesRegex(RuntimeError, 'one successful automatic attempt'):
            batch.verify_recompute(conn, source, gid, changed)


if __name__ == '__main__':
    unittest.main()
