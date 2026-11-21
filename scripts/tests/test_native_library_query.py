"""Verify the release gate against the catalog schema without a desktop build."""
import importlib.util
import json
from pathlib import Path
import sqlite3
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('library_query', ROOT / 'scripts/native-library-query-build-batch.py')
batch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(batch)


class LibraryQueryEvidence(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(':memory:')
        self.addCleanup(self.db.close)
        for name in ('001_baseline.sql', '002_interactions.sql', '004_capture_time.sql'):
            self.db.executescript((ROOT / 'src-tauri/migrations' / name).read_text())
        # The migration to v3 is inline in db::initialize.
        self.db.execute('CREATE TABLE query_contexts(fingerprint TEXT PRIMARY KEY, request_json TEXT NOT NULL, expires_at INTEGER NOT NULL)')
        self.source = Path('/owned/library-query-fixture')
        self.names = batch.MATCH + batch.OUTSIDE
        self.db.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('source','Owned',?,1,1)", (str(self.source),))
        self.db.executemany('''INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,indexed_at,updated_at)
            VALUES(?,'source',?,?,'tiff','image',137,1,1)''', [(name, name, name) for name in self.names])
        self.db.execute("UPDATE media_assets SET capture_local=?,capture_time_source='exif' WHERE id=?",
                        (batch.ORIGINAL_LOCAL, batch.TIME_SAMPLE))
        self.db.execute("INSERT INTO jobs(id,type,status,progress_done,progress_total,created_at,updated_at) VALUES('scan','SCAN','completed',130,130,1,1)")
        filter_json, sort_json = '{"keyword":"match-"}', '{"field":"filename","direction":"asc"}'
        self.db.execute("INSERT INTO selection_snapshots VALUES('frozen',?,?,'narrow',125,1,1000)", (filter_json, sort_json))
        self.db.execute("INSERT INTO selection_snapshot_items SELECT 'frozen',id FROM media_assets WHERE filename LIKE 'match-%'")
        self.db.executemany('INSERT INTO query_contexts VALUES(?,?,1000)', [
            ('narrow', '{"filter":' + filter_json + ',"sort":' + sort_json + '}'),
            ('wide', '{"filter":{},"sort":{"field":"filename","direction":"desc"}}'),
        ])
        self.narrow = dict(order=list(batch.MATCH), firstPage=120, frozenCount=124, excludedCount=1, scopeNotice=False)
        self.wide = dict(self.narrow, order=list(reversed(self.names)), scopeNotice=True)

    def apply_keep(self, ids):
        request = dict(target=dict(kind='snapshot', snapshotId='frozen', excludeAssetIds=[batch.EXCLUDED]),
                       patch={'pickState': 'keep'})
        self.db.execute("INSERT INTO change_batches VALUES('keep','review',?,2,NULL)", (json.dumps(request),))
        self.db.executemany("INSERT INTO change_batch_items VALUES('keep',?,'{}','{}')", [(name,) for name in ids])
        self.db.execute("UPDATE asset_reviews SET pick_state='keep',revision=1 WHERE asset_id IN (SELECT asset_id FROM change_batch_items)")

    def test_pages_and_frozen_scope_through_review_and_undo(self):
        baseline = batch.read_catalog(self.db)
        batch.verify('frozen', baseline, self.source, self.narrow, None)
        batch.verify('changed', baseline, self.source, self.wide, baseline)
        self.apply_keep(batch.MATCH[1:])
        applied = batch.read_catalog(self.db)
        batch.verify('applied', applied, self.source, {}, baseline)
        self.db.execute("UPDATE change_batches SET undone_at=3 WHERE id='keep'")
        self.db.execute("UPDATE asset_reviews SET pick_state='unreviewed',revision=2 WHERE revision=1")
        undone = batch.read_catalog(self.db)
        batch.verify('undone', undone, self.source, {}, applied)

        self.db.execute('''INSERT INTO capture_time_corrections VALUES(?,?,NULL,NULL,'exif',?,NULL,NULL,1,1,4)''',
                        (batch.TIME_SAMPLE, batch.ORIGINAL_LOCAL, batch.CORRECTED_LOCAL))
        self.db.execute("UPDATE media_assets SET capture_local=?,capture_time_source='manual' WHERE id=?",
                        (batch.CORRECTED_LOCAL, batch.TIME_SAMPLE))
        observed = dict(filename=batch.TIME_SAMPLE, displayedLocal=batch.CORRECTED_LOCAL)
        saved = batch.read_catalog(self.db)
        batch.verify('time-saved', saved, self.source, observed, undone)
        self.db.execute("UPDATE media_assets SET capture_utc_ms=1 WHERE id=?", (batch.TIME_SAMPLE,))
        with self.assertRaises(AssertionError):
            batch.verify('time-saved', batch.read_catalog(self.db), self.source, observed, undone)

        self.db.execute('UPDATE capture_time_corrections SET corrected_local=NULL,is_corrected=0,revision=2')
        self.db.execute("UPDATE media_assets SET capture_local=?,capture_utc_ms=NULL,capture_time_source='exif' WHERE id=?",
                        (batch.ORIGINAL_LOCAL, batch.TIME_SAMPLE))
        observed['displayedLocal'] = batch.ORIGINAL_LOCAL
        batch.verify('time-restored', batch.read_catalog(self.db), self.source, observed, saved)

    def test_duplicated_tail_does_not_pass_even_with_the_right_count(self):
        wrong = dict(self.wide, order=self.wide['order'][:-1] + [self.wide['order'][-2]])
        with self.assertRaises(AssertionError):
            batch.verify('changed', batch.read_catalog(self.db), self.source, wrong, None)

    def test_concealed_snapshot_replacement_does_not_pass(self):
        baseline = batch.read_catalog(self.db)
        self.db.execute("INSERT INTO selection_snapshots SELECT 'replacement',filter_json,sort_json,query_fingerprint,initial_count,created_at,expires_at FROM selection_snapshots")
        self.db.execute("INSERT INTO selection_snapshot_items SELECT 'replacement',asset_id FROM selection_snapshot_items WHERE snapshot_id='frozen'")
        self.db.execute("DELETE FROM selection_snapshots WHERE id='frozen'")
        with self.assertRaises(AssertionError):
            batch.verify('changed', batch.read_catalog(self.db), self.source, self.wide, baseline)

    def test_an_unchanged_write_count_cannot_hide_outside_writes(self):
        baseline = batch.read_catalog(self.db)
        self.apply_keep(batch.MATCH[1:-1] + batch.OUTSIDE[:1])
        with self.assertRaises(AssertionError):
            batch.verify('applied', batch.read_catalog(self.db), self.source, {}, baseline)


if __name__ == '__main__':
    unittest.main()
