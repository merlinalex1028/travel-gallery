"""Verify the release gate against the catalog schema without a desktop build."""
from contextlib import ExitStack, closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('library_query', ROOT / 'scripts/native-library-query-build-batch.py')
batch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(batch)


def pagination_frames(names, direction, *, scope=False):
    def frame(cards, loaded, selected=False):
        rows = ['Window: "旅图 · Travel Gallery", App: Travel Gallery Offline Bench.',
                '\t\t2 HTML content Description: 旅图 · Travel Gallery, URL: tauri://localhost#/library',
                f'\t\t\t\t48 button 改为{direction}',
                f'\t\t\t\t59 text · 当前查询 {len(names)} 张',
                f'\t\t\t\t77 text {loaded} 张已加载']
        if selected:
            rows.extend(['\t\t\t\t61 text 已选 124 张 冻结快照',
                         '\t\t\t\t72 text 已排除 1 张 冻结于 2026/11/23 11:37'])
            if scope:
                rows.append('\t\t\t\t74 text 筛选已改变；本次 124 张的冻结选区保持原范围。重新选择全部匹配才会更新范围。')
        for index, name in enumerate(cards):
            i = 81 + 6 * index
            rows.extend([f'\t\t\t\t{i} container (selected) {name}，未处理，0 星',
                         f'\t\t\t\t\t{i+1} image {name}', f'\t\t\t\t\t{i+2} text {name}'])
        return '\n'.join(rows)

    return dict(axFrames=[frame(names[start:start + 21], 120 if start < 100 else len(names))
                          for start in range(0, len(names), 20)],
                axSelection=frame(names[:15], len(names), selected=True))


class LibraryQueryEvidence(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(':memory:')
        self.addCleanup(self.db.close)
        for name in ('001_baseline.sql', '002_interactions.sql', '004_capture_time.sql'):
            self.db.executescript((ROOT / 'src-tauri/migrations' / name).read_text())
        # The migration to v3 is inline in db::initialize.
        self.db.execute('CREATE TABLE query_contexts(fingerprint TEXT PRIMARY KEY, request_json TEXT NOT NULL, expires_at INTEGER NOT NULL)')
        self.db.execute('CREATE TABLE source_options(source_id TEXT PRIMARY KEY REFERENCES source_roots(id), options_json TEXT NOT NULL)')
        self.source = Path('/owned/library-query-fixture')
        self.names = batch.MATCH + batch.OUTSIDE
        self.db.execute("INSERT INTO source_roots(id,display_name,absolute_path,created_at,updated_at) VALUES('source','Owned',?,1,1)", (str(self.source),))
        self.db.executemany('''INSERT INTO media_assets(id,source_id,relative_path,filename,extension,media_kind,byte_size,indexed_at,updated_at)
            VALUES(?,'source',?,?,'tiff','image',137,1,1)''', [(name, name, name) for name in self.names])
        self.db.execute("UPDATE media_assets SET capture_local=?,capture_time_source='exif' WHERE id=?",
                        (batch.ORIGINAL_LOCAL, batch.TIME_SAMPLE))
        self.db.execute('''UPDATE media_assets SET capture_local=?,capture_utc_ms=1712801700000,
            capture_tz_min=345,capture_time_source='exif' WHERE id=?''',
                        (f'{batch.KNOWN_ZONE_DATE} 08:00:00', batch.KNOWN_ZONE_SAMPLE))
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
        observed = dict(filename=batch.TIME_SAMPLE, displayedLocal=batch.CORRECTED_LOCAL,
                        knownZoneDate=batch.KNOWN_ZONE_DATE, knownZoneLabel='时区已知')
        saved = batch.read_catalog(self.db)
        batch.verify('time-saved', saved, self.source, observed, undone)
        with self.assertRaises(AssertionError):
            batch.verify('time-saved', saved, self.source, dict(observed, knownZoneLabel='媒体原始时间'), undone)
        self.db.execute("UPDATE media_assets SET capture_utc_ms=1 WHERE id=?", (batch.TIME_SAMPLE,))
        with self.assertRaises(AssertionError):
            batch.verify('time-saved', batch.read_catalog(self.db), self.source, observed, undone)

        self.db.execute('UPDATE capture_time_corrections SET corrected_local=NULL,is_corrected=0,revision=2')
        self.db.execute("UPDATE media_assets SET capture_local=?,capture_utc_ms=NULL,capture_time_source='exif' WHERE id=?",
                        (batch.ORIGINAL_LOCAL, batch.TIME_SAMPLE))
        restored = dict(filename=batch.TIME_SAMPLE, displayedLocal=batch.ORIGINAL_LOCAL)
        batch.verify('time-restored', batch.read_catalog(self.db), self.source, restored, saved)

    def test_duplicated_tail_does_not_pass_even_with_the_right_count(self):
        wrong = dict(self.wide, order=self.wide['order'][:-1] + [self.wide['order'][-2]])
        with self.assertRaises(AssertionError):
            batch.verify('changed', batch.read_catalog(self.db), self.source, wrong, None)

    def test_ax_orders_read_only_card_labels_and_combine_adjacent_frames(self):
        for step, evidence, expected in [
            ('frozen', pagination_frames(batch.MATCH, '降序'), self.narrow),
            ('changed', pagination_frames(self.names[::-1], '升序', scope=True), self.wide),
        ]:
            with self.subTest(step=step):
                self.assertEqual(batch.pagination_observation(step, evidence), expected)

    def test_ax_rejects_incomplete_and_inconsistent_capture(self):
        evidence = pagination_frames(batch.MATCH, '降序')
        frames, selection = evidence['axFrames'], evidence['axSelection']
        for altered, problem in [
            (self.narrow, 'Supply ordered'),
            (dict(evidence, axFrames=[frames[0], *frames[2:]]), 'must overlap'),
            (dict(evidence, axFrames=frames[:-1]), 'entire paginated query'),
            (dict(evidence, axFrames=[frames[0], frames[1].replace('match-021.tiff', 'match-020.tiff'), *frames[2:]]),
             'duplicate cards'),
            (dict(evidence, axFrames=[frames[0], frames[1].replace('match-021.tiff', 'match-019.tiff'), *frames[2:]]),
             'order disagrees'),
            (dict(evidence, axFrames=[frames[0], *frames[1:-1], frames[-1].replace('125 张已加载', '120 张已加载')]),
             'counts changed inconsistently'),
            (dict(evidence, axSelection=selection.replace('已排除 1 张', '排除')), 'excluded count'),
            (dict(evidence, axSelection=selection.replace('match-001.tiff', 'outside-000.tiff')),
             'Selection frame disagrees'),
        ]:
            with self.subTest(problem=problem), self.assertRaisesRegex(ValueError, problem):
                batch.pagination_observation('frozen', altered)

    def test_ax_requires_unambiguous_native_query_and_selection_labels(self):
        evidence = pagination_frames(self.names[::-1], '升序', scope=True)
        for old, new in [
            ('App: Travel Gallery Offline Bench.', 'App: Travel Gallery.'),
            ('HTML content', 'image'),
            ('改为升序', '改为降序'),
            ('当前查询 130 张', '当前查询 125 张'),
            ('130 张已加载', '120 张已加载'),
            ('已选 124 张 冻结快照', '已选 124 张'),
            ('已排除 1 张', '已排除 2 张\n\t\t\t\t75 text 已排除 1 张'),
            ('本次 124 张的冻结选区', '本次 125 张的冻结选区'),
            ('筛选已改变', '查询结果尚未就绪'),
        ]:
            with self.subTest(old=old), self.assertRaises(ValueError):
                batch.pagination_observation('changed', dict(
                    evidence, axSelection=evidence['axSelection'].replace(old, new)))
        for frames in ([], [evidence['axFrames'][0]], [None, None]):
            with self.subTest(frames=frames), self.assertRaises(ValueError):
                batch.pagination_observation('changed', dict(evidence, axFrames=frames))
        with self.assertRaises(ValueError):
            batch.pagination_observation('changed', dict(evidence, axSelection=None))

    def test_premature_frozen_submissions_retry_without_advancing_or_publishing(self):
        with (tempfile.TemporaryDirectory(prefix='travel-gallery-query-controller-') as temporary,
              ExitStack() as stack):
            work = Path(temporary).resolve()
            data = work / 'data'
            data.mkdir()
            db_path = data / 'catalog.sqlite'
            fixture = work / 'fixture'
            self.db.execute('UPDATE source_roots SET absolute_path=?', (str(fixture / 'source'),))
            self.db.commit()
            db = stack.enter_context(closing(sqlite3.connect(db_path)))
            with db:
                db.executescript('\n'.join(self.db.iterdump()))
                db.execute('DELETE FROM selection_snapshot_items')
                db.execute('DELETE FROM selection_snapshots')
                db.execute("UPDATE jobs SET status='running' WHERE id='scan'")

            state = dict(normalBefore={}, sourceBefore=[])
            events = []
            frozen_ax = pagination_frames(batch.MATCH, '降序')

            def submit(step, **observed):
                pending = work / 'control/pending'
                pending.write_text(json.dumps(dict(step=step, **observed)))
                pending.replace(work / 'control/step.json')

            def event(stage, **details):
                events.append((stage, details.get('step')))
                if stage == 'awaitingNativeCheckpoint' and details['step'] == 'frozen':
                    submit('finish')
                elif stage == 'nativeCheckpointNotReady':
                    assert state['expectedCheckpoint'] == 'frozen'
                    if details['reason'].startswith('Invalid request'):
                        submit('frozen', **frozen_ax)
                    elif details['reason'].startswith('Background work'):
                        with db:
                            db.execute("UPDATE jobs SET status='completed' WHERE id='scan'")
                        submit('frozen', **frozen_ax)
                    elif details['reason'].startswith('No frozen selection'):
                        with db:
                            db.execute('INSERT INTO selection_snapshots VALUES(?,?,?,?,?,?,?)',
                                       self.db.execute('SELECT * FROM selection_snapshots').fetchone())
                            db.executemany('INSERT INTO selection_snapshot_items VALUES(?,?)',
                                           self.db.execute('SELECT * FROM selection_snapshot_items'))
                        submit('frozen', **self.narrow)
                    elif details['reason'].startswith('Supply ordered'):
                        submit('frozen', **frozen_ax)
                elif stage == 'nativeLibraryQueryCheckpoint' and details['step'] == 'frozen':
                    self.assertEqual(details['observedNames'], 125)
                    self.assertEqual(details['snapshots'], 1)

            with (patch.multiple(batch.b, WORK=work, DATA=data, STATE=state, event=event),
                  patch.object(batch.b, 'check', return_value={}),
                  patch.object(batch.reuse, 'normal_state', return_value={}),
                  patch.object(batch.reuse, 'source_state', return_value=[]),
                  patch.object(batch.picker, 'verified_process', return_value=42),
                  patch.object(batch, 'STEPS', ('frozen', 'changed', 'finish')),
                  patch.object(batch.time, 'monotonic', side_effect=[0, 1, 2, 3, 4, 5, 0, 600]),
                  patch.object(batch.exact, 'publish') as publish):
                with self.assertRaisesRegex(TimeoutError, 'No accepted native checkpoint: changed'):
                    batch.accept(work / 'candidate.app', fixture, {}, {})
                publish.assert_not_called()

            self.assertEqual(events, [
                ('readyForNativeGui', None), ('awaitingNativeCheckpoint', 'frozen'),
                *[('nativeCheckpointNotReady', 'frozen')] * 4,
                ('nativeLibraryQueryCheckpoint', 'frozen'), ('awaitingNativeCheckpoint', 'changed'),
            ])

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

    def restore_states(self):
        self.db.execute('CREATE TABLE app_meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL)')
        self.db.execute("INSERT INTO app_meta VALUES('epoch', 130)")
        backup = batch.read_restore_state(self.db)
        batch.verify_restore('backed-up', backup, {'backupCompleted': True}, backup, backup)
        self.db.execute("UPDATE app_meta SET value=131 WHERE key='epoch'")
        self.db.execute('UPDATE asset_reviews SET rating=5,revision=1 WHERE asset_id=?', (batch.EXCLUDED,))
        before = dict(review=dict(pickState='unreviewed', rating=0, isFavorite=False, note='', revision=0, updatedAt=1),
                      tagIds=[])
        after = dict(review=dict(before['review'], rating=5, revision=1, updatedAt=2), tagIds=[])
        request = dict(target=dict(kind='explicit', assetIds=[batch.EXCLUDED]), patch={'rating': 5},
                       clientMutationId='rating-once')
        self.db.execute("INSERT INTO change_batches VALUES('rating','review',?,2,NULL)", (json.dumps(request),))
        self.db.execute("INSERT INTO change_batch_items VALUES('rating',?,?,?)",
                        (batch.EXCLUDED, json.dumps(before), json.dumps(after)))
        receipt = dict(changeBatchId='rating', affectedCount=1, unchangedCount=0, conflicts=[])
        self.db.execute("INSERT INTO mutation_receipts VALUES('rating-once','hash',?,2)", (json.dumps(receipt),))
        changed = batch.read_restore_state(self.db)
        batch.verify_restore('rating-changed', changed, dict(filename=batch.EXCLUDED, rating=5), backup, backup)
        return backup, changed

    def test_restore_requires_a_new_epoch_and_an_exact_automatic_backup(self):
        backup, changed = self.restore_states()
        observed = dict(filename=batch.EXCLUDED, rating=0, restored=True)
        restored = dict(backup, epoch=132)
        batch.verify_restore('backup-restored', restored, observed, backup, backup, changed, changed)
        for state, rollback in [(dict(backup, epoch=131), changed), (restored, backup),
                                (dict(restored, assets=changed['assets']), changed)]:
            with self.assertRaises(AssertionError):
                batch.verify_restore('backup-restored', state, observed, backup, backup, changed, rollback)

    def test_restore_rejects_changed_effective_dates_roots_organization_and_history(self):
        backup, changed = self.restore_states()
        self.db.execute('UPDATE asset_reviews SET rating=0,revision=0 WHERE asset_id=?', (batch.EXCLUDED,))
        self.db.execute("UPDATE app_meta SET value=132 WHERE key='epoch'")
        self.db.execute("DELETE FROM change_batch_items WHERE batch_id='rating'")
        self.db.execute("DELETE FROM change_batches WHERE id='rating'")
        self.db.execute("DELETE FROM mutation_receipts WHERE client_mutation_id='rating-once'")
        observed = dict(filename=batch.EXCLUDED, rating=0, restored=True)
        restored = batch.read_restore_state(self.db)
        batch.verify_restore('backup-restored', restored, observed, backup, backup, changed, changed)
        for statement in [
            "UPDATE media_assets SET capture_local='2030-01-01 00:00:00' WHERE filename='outside-000.tiff'",
            "UPDATE source_roots SET absolute_path='/wrong-source'",
            "INSERT INTO tags VALUES('unexpected','unexpected',NULL,1)",
            "INSERT INTO source_options VALUES('source','{\"recursive\":false}')",
            "INSERT INTO change_batches VALUES('unexpected','review','{}',3,NULL)",
        ]:
            with self.subTest(statement=statement):
                self.db.execute('SAVEPOINT altered')
                self.db.execute(statement)
                with self.assertRaises(AssertionError):
                    batch.verify_restore('backup-restored', batch.read_restore_state(self.db), observed,
                                         backup, backup, changed, changed)
                self.db.execute('ROLLBACK TO altered')
                self.db.execute('RELEASE altered')

    def test_rating_requires_one_exact_history_item_and_receipt(self):
        backup, changed = self.restore_states()
        for statement in [
            "DELETE FROM mutation_receipts",
            "UPDATE change_batch_items SET asset_id='outside-000.tiff'",
            "UPDATE change_batches SET payload_json='{\"target\":{\"kind\":\"explicit\",\"assetIds\":[\"outside-000.tiff\"]},\"patch\":{\"rating\":5}}'",
        ]:
            with self.subTest(statement=statement):
                self.db.execute('SAVEPOINT altered')
                self.db.execute(statement)
                with self.assertRaises(AssertionError):
                    batch.verify_restore('rating-changed', batch.read_restore_state(self.db),
                                         dict(filename=batch.EXCLUDED, rating=5), backup, backup)
                self.db.execute('ROLLBACK TO altered')
                self.db.execute('RELEASE altered')

    def test_restore_accounts_only_for_the_exact_source_offline_transition(self):
        backup, changed = self.restore_states()
        availability = dict(roots=[('source', 0)],
                            assets=[(aid, 'offline') for aid, _ in backup['availability']['assets']])
        observed = dict(filename=batch.EXCLUDED, rating=0, restored=True)
        restored = dict(backup, epoch=133, availability=availability)
        batch.verify_restore('backup-restored', restored, observed, backup, backup, changed, changed)
        with self.assertRaises(AssertionError):
            # The old restore epoch plus a status bump must not conceal epoch reuse.
            batch.verify_restore('backup-restored', dict(restored, epoch=132), observed,
                                 backup, backup, changed, changed)
        invalid = dict(availability, assets=[(aid, 'missing') for aid, _ in availability['assets']])
        with self.assertRaises(AssertionError):
            batch.verify_restore('backup-restored', dict(restored, availability=invalid), observed,
                                 backup, backup, changed, changed)

    def test_map_requires_all_three_exact_gps_records_and_visible_cell_counts(self):
        for name, lat, lon in batch.MAP_GPS:
            self.db.execute('UPDATE media_assets SET gps_lat=?,gps_lon=? WHERE filename=?', (lat, lon, name))
        rows = batch.read_map_locations(self.db)
        observed = dict(mapRendered=True, mapErrorVisible=False, cells=[
            dict(lat=43.47, lon=11.88, count=2), dict(lat=43.5, lon=11.92, count=1)])
        batch.verify_map(rows, observed)
        for locations, ui in [(rows[:-1], observed),
                              (rows, dict(observed, cells=[dict(lat=43.47, lon=11.88, count=3)])),
                              (rows, dict(observed, mapErrorVisible=True))]:
            with self.assertRaises(AssertionError):
                batch.verify_map(locations, ui)


if __name__ == '__main__':
    unittest.main()
