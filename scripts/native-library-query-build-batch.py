"""Accept one approved macOS candidate against the native gallery and its isolated catalog."""
from contextlib import closing
from datetime import datetime, timezone
import fcntl
import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import time

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'candidate', Path(__file__).with_name('native-story-version-build-batch.py'))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)
batch, b, reuse, picker, exact = build.batch, build.b, build.reuse, build.picker, build.exact
tiny_tiff = build.load('native-sorting-batch').tiny_tiff
BATCH = 'library-query'
b.APPROVAL = batch.ROOT / 'docs/native-library-query-budget.json'
b.RECEIPT = batch.ROOT / 'docs/native-library-query-latest.json'
EPOCH_BATCH = 'catalog-epoch'
MATCH = tuple(f'match-{i:03}.tiff' for i in range(125))
OUTSIDE = tuple(f'outside-{i:03}.tiff' for i in range(5))
EXCLUDED = MATCH[0]
TIME_SAMPLE = OUTSIDE[0]
ORIGINAL_LOCAL = '2024-04-10 08:00:00'
CORRECTED_LOCAL = '2025-05-11 09:30:00'
KNOWN_ZONE_SAMPLE = OUTSIDE[1]
KNOWN_ZONE_DATE = '2024-04-11'
STEPS = ('empty', 'frozen', 'changed', 'applied', 'undone', 'time-saved', 'time-restored', 'finish')
RESTORE_STEPS = ('backed-up', 'rating-changed', 'backup-restored')
RESTORE_STABLE_KEYS = ('roots', 'times', 'corrections', 'snapshots', 'members', 'organization')
RESTORE_ORGANIZATION_TABLES = ('tags', 'asset_tags', 'linked_media', 'collections',
                              'collection_assets', 'saved_filters', 'source_options', 'app_settings')


def write_sources(source):
    for index, name in enumerate(MATCH + OUTSIDE):
        capture = ORIGINAL_LOCAL.replace('-', ':') if name == TIME_SAMPLE else None
        offset = None
        if name == KNOWN_ZONE_SAMPLE:
            capture = f"{KNOWN_ZONE_DATE.replace('-', ':')} 08:00:00"
            offset = '+05:45'
        (source / name).write_bytes(tiny_tiff(index, capture, offset))


def read_catalog(db):
    assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
    assert not db.execute('PRAGMA foreign_key_check').fetchone()
    assert not db.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
    return dict(
        roots=db.execute('SELECT id,absolute_path,is_online,is_enabled FROM source_roots').fetchall(),
        assets=db.execute('''SELECT a.filename,a.id,a.source_id,a.relative_path,a.file_state,
            r.pick_state,r.rating,r.is_favorite,r.note,r.revision FROM media_assets a
            JOIN asset_reviews r ON r.asset_id=a.id ORDER BY a.filename COLLATE NOCASE,a.id''').fetchall(),
        scans=db.execute('''SELECT status,progress_done,progress_total,failure_count FROM jobs
            WHERE type='SCAN' ORDER BY created_at,id''').fetchall(),
        snapshots=db.execute('''SELECT id,json_extract(filter_json,'$.keyword'),
            json_extract(sort_json,'$.field'),json_extract(sort_json,'$.direction'),initial_count
            FROM selection_snapshots ORDER BY id''').fetchall(),
        members=db.execute('SELECT snapshot_id,asset_id FROM selection_snapshot_items ORDER BY 1,2').fetchall(),
        contexts=db.execute("""SELECT json_extract(request_json,'$.filter.keyword'),
            json_extract(request_json,'$.sort.field'),json_extract(request_json,'$.sort.direction')
            FROM query_contexts""").fetchall(),
        changes=db.execute('SELECT id,action,payload_json,undone_at FROM change_batches').fetchall(),
        changed_ids=db.execute('SELECT batch_id,asset_id FROM change_batch_items ORDER BY 1,2').fetchall(),
        times=db.execute('''SELECT filename,capture_local,capture_utc_ms,capture_tz_min,capture_time_source
            FROM media_assets ORDER BY filename COLLATE NOCASE,id''').fetchall(),
        corrections=db.execute('''SELECT asset_id,original_local,original_utc_ms,original_tz_min,original_source,
            corrected_local,corrected_utc_ms,corrected_tz_min,is_corrected,revision
            FROM capture_time_corrections''').fetchall(),
    )


def read_restore_state(db):
    return dict(
        epoch=db.execute("SELECT value FROM app_meta WHERE key='epoch'").fetchone()[0],
        roots=db.execute('SELECT id,absolute_path,volume_id,is_enabled FROM source_roots ORDER BY id').fetchall(),
        assets=db.execute('''SELECT a.id,a.filename,a.source_id,a.relative_path,r.pick_state,r.rating,
            r.is_favorite,r.note,r.revision FROM media_assets a JOIN asset_reviews r ON r.asset_id=a.id
            ORDER BY a.filename COLLATE NOCASE,a.id''').fetchall(),
        times=db.execute('''SELECT id,capture_local,capture_utc_ms,capture_tz_min,capture_time_source
            FROM media_assets ORDER BY id''').fetchall(),
        corrections=db.execute('SELECT * FROM capture_time_corrections ORDER BY asset_id').fetchall(),
        snapshots=db.execute('SELECT * FROM selection_snapshots ORDER BY id').fetchall(),
        members=db.execute('SELECT * FROM selection_snapshot_items ORDER BY 1,2').fetchall(),
        organization={table: db.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall()
                      for table in RESTORE_ORGANIZATION_TABLES},
        history={table: db.execute(f'SELECT * FROM {table} ORDER BY 1,2').fetchall()
                 for table in ('change_batches', 'change_batch_items', 'mutation_receipts')},
    )


def read_backup(path):
    assert path.is_file() and not path.is_symlink()
    with closing(sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('BEGIN')
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not db.execute('PRAGMA foreign_key_check').fetchone()
        return read_restore_state(db)


def verify_restore(step, current, observations, backup, baseline, changed=None, rollback=None):
    if step == 'backed-up':
        assert current == backup == baseline
        assert observations == {'backupCompleted': True}
    elif step == 'rating-changed':
        assert observations == dict(filename=EXCLUDED, rating=5)
        assert current['epoch'] > backup['epoch']
        index = next(i for i, row in enumerate(backup['assets']) if row[1] == EXCLUDED)
        expected = list(backup['assets'])
        old = expected[index]
        assert old[5] == 0
        expected[index] = (*old[:5], 5, *old[6:8], old[8] + 1)
        assert current['assets'] == expected
        assert all(current[key] == backup[key] for key in RESTORE_STABLE_KEYS)
        added = {}
        for table, rows in backup['history'].items():
            actual = current['history'][table]
            assert set(rows) <= set(actual) and len(actual) == len(rows) + 1
            [added[table]] = set(actual) - set(rows)
        change_id, action, raw, _, undone = added['change_batches']
        request = json.loads(raw)
        assert action == 'review' and undone is None
        assert request['target'] == dict(kind='explicit', assetIds=[old[0]])
        assert request['patch'] == {'rating': 5}
        batch_id, asset_id, before, after = added['change_batch_items']
        assert (batch_id, asset_id) == (change_id, old[0])
        before, after = json.loads(before), json.loads(after)
        tags = [tag for aid, tag in backup['organization']['asset_tags'] if aid == old[0]]
        assert before['tagIds'] == after['tagIds'] == sorted(tags)
        previous = dict(pickState=old[4], rating=old[5], isFavorite=bool(old[6]), note=old[7], revision=old[8])
        assert {k: v for k, v in before['review'].items() if k != 'updatedAt'} == previous
        assert {k: v for k, v in after['review'].items() if k != 'updatedAt'} == dict(previous, rating=5, revision=old[8] + 1)
        mutation, _, receipt, _ = added['mutation_receipts']
        assert mutation == request['clientMutationId']
        assert json.loads(receipt) == dict(changeBatchId=change_id, affectedCount=1, unchangedCount=0, conflicts=[])
    else:
        assert step == 'backup-restored'
        assert observations == dict(filename=EXCLUDED, rating=0, restored=True)
        assert rollback == changed
        assert current['epoch'] == max(changed['epoch'], backup['epoch']) + 1
        assert all(current[key] == backup[key] for key in ('assets', 'history', *RESTORE_STABLE_KEYS))


def verify(step, state, source, observed, previous):
    if step == 'empty':
        assert observed == {} and all(not state[key] for key in
            ('roots', 'assets', 'scans', 'snapshots', 'members', 'changes', 'changed_ids', 'times', 'corrections'))
        return

    [(source_id, root, online, enabled)] = state['roots']
    assert (root, online, enabled) == (str(source), 1, 1)
    assets = state['assets']
    assert tuple(row[0] for row in assets) == MATCH + OUTSIDE
    ids = {row[0]: row[1] for row in assets}
    assert all((sid, path, availability) == (source_id, name, 'available')
               for name, _, sid, path, availability, *_ in assets)
    assert state['scans'] and set(state['scans']) == {('completed', len(assets), len(assets), 0)}
    [(snapshot_id, keyword, field, direction, count)] = state['snapshots']
    assert (keyword, field, direction, count) == ('match-', 'filename', 'asc', len(MATCH))
    assert set(state['members']) == {(snapshot_id, ids[name]) for name in MATCH}
    if previous:
        for key in ('roots', 'snapshots', 'members'):
            assert state[key] == previous[key], key
        assert [row[:5] for row in assets] == [row[:5] for row in previous['assets']]

    time_steps = ('time-saved', 'time-restored')
    if step in time_steps:
        assert previous and state['changes'] == previous['changes'] and state['changed_ids'] == previous['changed_ids']
        saved = step == 'time-saved'
        expected = (CORRECTED_LOCAL, None, None, 'manual') if saved else (ORIGINAL_LOCAL, None, None, 'exif')
        expected_observation = dict(filename=TIME_SAMPLE, displayedLocal=expected[0])
        if saved:
            expected_observation.update(knownZoneDate=KNOWN_ZONE_DATE, knownZoneLabel='时区已知')
            known = next(row[1:] for row in state['times'] if row[0] == KNOWN_ZONE_SAMPLE)
            assert known[0] == f'{KNOWN_ZONE_DATE} 08:00:00' and known[1] is not None and known[2:] == (345, 'exif')
        assert observed == expected_observation
        assert state['corrections'] == [(ids[TIME_SAMPLE], ORIGINAL_LOCAL, None, None, 'exif',
                                        CORRECTED_LOCAL if saved else None, None, None, int(saved), 1 if saved else 2)]
        assert state['times'] == [(name, *(expected if name == TIME_SAMPLE else fields))
                                  for name, *fields in previous['times']]
    else:
        assert not state['corrections']
        assert next(row[1:] for row in state['times'] if row[0] == TIME_SAMPLE) == (ORIGINAL_LOCAL, None, None, 'exif')

    if step in ('frozen', 'changed'):
        order = [row[0] for row in assets if row[0].startswith('match-')] if step == 'frozen' else [
            row[0] for row in reversed(assets)]
        assert observed == dict(order=order, firstPage=120, frozenCount=len(MATCH) - 1,
                                excludedCount=1, scopeNotice=step == 'changed')
        condition = ('match-', 'filename', 'asc') if step == 'frozen' else (None, 'filename', 'desc')
        assert condition in state['contexts']
        assert not state['changes'] and not state['changed_ids']
    elif step not in time_steps:
        assert observed == {}
        [(change_id, kind, raw, undone_at)] = state['changes']
        assert kind == 'review' and (undone_at is not None) == (step == 'undone')
        request = json.loads(raw)
        assert request['target'] == dict(kind='snapshot', snapshotId=snapshot_id,
                                         excludeAssetIds=[ids[EXCLUDED]])
        assert request['patch'] == {'pickState': 'keep'}
        assert set(state['changed_ids']) == {(change_id, ids[name]) for name in MATCH[1:]}

    for name, *_, pick, rating, favorite, note, revision in assets:
        affected = name in MATCH[1:] and step in ('applied', 'undone', *time_steps)
        expected_pick = 'keep' if affected and step == 'applied' else 'unreviewed'
        expected_revision = (1 if step == 'applied' else 2) if affected else 0
        assert (pick, rating, favorite, note, revision) == (expected_pick, 0, 0, '', expected_revision), name


def accept(candidate, fixture, release, env, *, check_restore=False):
    control = b.WORK / 'control'
    control.mkdir()
    source = (fixture / 'source').resolve()
    database = b.DATA / 'catalog.sqlite'
    backup_dir = b.WORK / 'backup'
    if check_restore:
        backup_dir.mkdir()
        b.STATE['backupDirectory'] = str(backup_dir)
    b.STATE.update(status='ready', application=str(b.APP_PATH), expectedDatabase=str(database),
                   controlDirectory=str(control))
    b.event('readyForNativeGui', source=str(source), **b.check())
    pid, previous = None, None
    baseline, backup, changed = None, None, None
    regular_steps = STEPS[:-1]
    for step in (*regular_steps, *(RESTORE_STEPS if check_restore else ()), 'finish'):
        request = control / 'step.json'
        deadline = time.monotonic() + 600
        while not request.is_file():
            b.check()
            if time.monotonic() >= deadline:
                raise TimeoutError(f'No native checkpoint: {step}')
            time.sleep(.25)
        observed = json.loads(request.read_text())
        request.unlink()
        assert observed.pop('step') == step
        assert reuse.normal_state() == b.STATE['normalBefore']
        assert reuse.source_state() == b.STATE['sourceBefore']
        if step == 'finish':
            assert not observed and exact.exited(b.APP_PATH) and exact.exited(b.NORMAL)
            assert batch.verify_inputs() == b.STATE['sourceCommit']
            exact.check_release(release, b.NORMAL)
            exact.publish(candidate, release, env)
            b.STATE.update(status='finished', qaQuitNormally=True)
            return
        current_pid = picker.verified_process(database)
        assert pid is None or current_pid == pid
        pid = current_pid
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
            db.execute('BEGIN')
            state = read_catalog(db)
            if check_restore and step in ('time-restored', *RESTORE_STEPS):
                restore_state = read_restore_state(db)
            if step == 'backed-up':
                completed_backups = db.execute("""SELECT status,progress_done,progress_total,failure_count
                    FROM jobs WHERE type='BACKUP'""").fetchall()
        if step in RESTORE_STEPS:
            [selected] = list(backup_dir.glob('travel-gallery-*.sqlite'))
            if step == 'backed-up':
                backup = read_backup(selected)
                assert completed_backups == [('completed', 1, 1, 0)]
            assert read_backup(selected) == backup
            rollback = None
            if step == 'backup-restored':
                [auto_backup] = list(b.DATA.glob('before-restore-*.sqlite'))
                rollback = read_backup(auto_backup)
                assert not list(b.DATA.glob('restore-staging-*'))
            verify_restore(step, restore_state, observed, backup, baseline, changed, rollback)
            if step == 'rating-changed':
                changed = restore_state
            b.event('nativeCatalogEpochCheckpoint', step=step, catalogEpoch=restore_state['epoch'],
                    backupEpoch=backup['epoch'], **observed)
            continue
        try:
            verify(step, state, source, observed, previous)
        except (AssertionError, ValueError) as error:
            raise RuntimeError(f'Library query checkpoint rejected: {step}: {error or "catalog or GUI mismatch"}') from error
        if step != 'empty':
            previous = state
        if check_restore and step == 'time-restored':
            baseline = restore_state
        order = observed.pop('order', [])
        b.event('nativeLibraryQueryCheckpoint', step=step, assets=len(state['assets']),
                snapshots=len(state['snapshots']), observedNames=len(order),
                firstName=order[0] if order else None, lastName=order[-1] if order else None,
                reviewed=len(state['changed_ids']), **observed)


def measure_query_latency(env):
    b.command('catalogQueryLatency', ['node', 'scripts/native.mjs', 'cargo', 'test', '--locked',
              '--manifest-path', 'src-tauri/Cargo.toml', '--test', 'performance', '--',
              '--ignored', '--exact', 'fifty_thousand_record_baseline', '--nocapture'],
              dict(env, TRAVEL_GALLERY_BENCH_COUNT='50000', TRAVEL_GALLERY_PERF_OUTPUT=str(b.WORK / 'query-latency.json')))
    stats = json.loads((b.WORK / 'query-latency.json').read_text())
    assert (stats['recordCount'], stats['queryRuns'], stats['firstPageRecords']) == (50_000, 100, 120)
    b.event('catalogQueryLatencyMeasured', meets300msInTestProfile=stats['queryP95Ms'] <= 300,
            **{key: stats[key] for key in (
        'recordCount', 'queryRuns', 'buildProfile', 'queryP50Ms', 'queryP95Ms', 'neighborP95Ms', 'snapshotMs')})


def run(approval):
    check_restore = approval['batch'] == EPOCH_BATCH
    candidate, fixture, release, env = build.build_candidate(
        approval, approval['batch'], 'CatalogEpoch' if check_restore else 'LibraryQuery',
        write_sources, picker_only=True,
        after_regressions=measure_query_latency if check_restore else None)
    preparation = json.loads((b.APP_PATH.parent / 'preparation.json').read_text())
    assert preparation['pickerOnly']
    assert not any('absolute-path' in name for name in preparation['parentEntitlements'])
    accept(candidate, fixture, release, env, check_restore=check_restore)


if __name__ == '__main__':
    if sys.argv[1:] not in ([], ['--catalog-epoch']):
        raise SystemExit('Usage: native-library-query-build-batch.py [--catalog-epoch]')
    batch_name = EPOCH_BATCH if sys.argv[1:] else BATCH
    if batch_name == EPOCH_BATCH:
        b.APPROVAL = batch.ROOT / 'docs/native-catalog-epoch-budget.json'
        b.RECEIPT = batch.ROOT / 'docs/native-catalog-epoch-latest.json'
    if not b.APPROVAL.is_file() or b.APPROVAL.is_symlink():
        raise SystemExit(f'Fresh {batch_name} batch approval is required; no temporary files generated')
    batch.run = run
    with b.APPROVAL.open('r+') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        approval = json.load(handle)
        if (approval.get('batch') != batch_name or approval.get('status') != 'active'
                or approval.get('userAnswer') != '允许这一轮，结束即清理' or approval.get('claimedAt')
                or approval.get('limits') != batch.LIMITS
                or approval.get('requiredFreeBytes') != batch.LIMITS['logicalBytes']):
            raise SystemExit(f'This candidate needs fresh, unclaimed {batch_name} approval')
        approval['claimedAt'] = datetime.now(timezone.utc).isoformat()
        handle.seek(0)
        json.dump(approval, handle, ensure_ascii=False, indent=2)
        handle.truncate()
        handle.flush()
        raise SystemExit(batch.main(approval))
