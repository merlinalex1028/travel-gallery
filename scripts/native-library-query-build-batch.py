"""Accept one approved macOS candidate against the native gallery and its isolated catalog."""
from contextlib import closing, contextmanager
from datetime import datetime, timezone
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import time

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'candidate', Path(__file__).with_name('native-story-version-build-batch.py'))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)
batch, b, reuse, picker, exact = build.batch, build.b, build.reuse, build.picker, build.exact
native_gui = build.load('native-gui-preflight')
tiny_tiff = build.load('native-sorting-batch').tiny_tiff
BATCH = 'library-query'
b.APPROVAL = batch.ROOT / 'docs/native-library-query-budget.json'
b.RECEIPT = batch.ROOT / 'docs/native-library-query-latest.json'
EPOCH_BATCH = 'catalog-epoch'
EPOCH_AX_BATCH = 'catalog-epoch-ax'
DEADLINE_BATCH = 'catalog-snapshot-deadline'
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
RESTORE_VERSION_STEPS = ('backup-reviewing', 'backup-version-rejected')
MAP_STEP = 'map-rendered'
EXPIRY_STEPS = ('expiry-frozen', 'expiry-rejected', 'expiry-refreshed', 'expiry-reselected')
SNAPSHOT_TTL_MS = 86_400_000
MAP_GPS = ((OUTSIDE[2], 43.47, 11.88), (OUTSIDE[3], 43.47, 11.88), (OUTSIDE[4], 43.5, 11.92))
RESTORE_STABLE_KEYS = ('roots', 'times', 'locations', 'corrections', 'snapshots', 'members', 'organization')
RESTORE_ORGANIZATION_TABLES = ('tags', 'asset_tags', 'linked_media', 'collections',
                              'collection_assets', 'saved_filters', 'source_options', 'app_settings')


def write_sources(source, *, include_gps=False):
    for index, name in enumerate(MATCH + OUTSIDE):
        capture = ORIGINAL_LOCAL.replace('-', ':') if name == TIME_SAMPLE else None
        offset = None
        if name == KNOWN_ZONE_SAMPLE:
            capture = f"{KNOWN_ZONE_DATE.replace('-', ':')} 08:00:00"
            offset = '+05:45'
        (source / name).write_bytes(tiny_tiff(index, capture, offset))
    if include_gps:
        add_gps = build.load('native-map-trip-batch').add_gps
        for name, lat, lon in [
            (OUTSIDE[2], (43, 28, 12), (11, 52, 48)),
            (OUTSIDE[3], (43, 28, 12), (11, 52, 48)),
            (OUTSIDE[4], (43, 30, 0), (11, 55, 12)),
        ]:
            path = source / name
            path.write_bytes(add_gps(path.read_bytes(), lat, lon))


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
        locations=db.execute('SELECT id,gps_lat,gps_lon FROM media_assets ORDER BY id').fetchall(),
        availability=dict(roots=db.execute('SELECT id,is_online FROM source_roots ORDER BY id').fetchall(),
                          assets=db.execute('SELECT id,file_state FROM media_assets ORDER BY id').fetchall()),
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


def prepare_backup_version_probe(original, probe):
    assert probe.parent == b.WORK and not original.is_symlink()
    with original.open('rb') as source, probe.open('xb') as target:
        shutil.copyfileobj(source, target)
    with closing(sqlite3.connect(probe.as_uri() + '?mode=rw', uri=True)) as db:
        assert db.execute('PRAGMA journal_mode=DELETE').fetchone() == ('delete',)


def rewrite_backup_version_probe(probe):
    assert probe.parent == b.WORK and probe.is_file() and probe.resolve() == probe
    before = probe.stat()
    with closing(sqlite3.connect(probe.as_uri() + '?mode=rw', uri=True)) as db, db:
        changed = db.execute('''UPDATE asset_reviews SET rating=4 WHERE rating=0 AND asset_id IN
            (SELECT id FROM media_assets WHERE filename=?)''', (EXCLUDED,))
        assert changed.rowcount == 1
    os.utime(probe, ns=(before.st_atime_ns, before.st_mtime_ns))
    after = probe.stat()
    assert (after.st_size, after.st_mtime_ns) == (before.st_size, before.st_mtime_ns)


def verify_backup_version_observation(step, evidence, probe):
    frame = evidence.get('axFrame')
    assert set(evidence) == {'axFrame'} and isinstance(frame, str)
    assert frame.startswith('Window: ') and 'App: Travel Gallery Offline Bench.' in frame.split('\n', 1)[0]
    assert re.search(AX_ROW + r'HTML content .*URL: tauri://localhost#/settings$', frame)
    if step == 'backup-reviewing':
        assert str(probe) in frame
        assert re.search(AX_ROW + r'button 备份当前库并恢复(?:$|\s)', frame)
    else:
        assert '备份文件在确认前已变化，请重新选择' in frame


def verify_restored_state(current, backup, changed, rollback):
    assert rollback == changed
    # Status polling may mark the source offline after restore revokes app grants.
    status_transition = 0
    if current['availability'] != backup['availability']:
        [(source_id, online)] = backup['availability']['roots']
        assert online == 1
        expected = dict(roots=[(source_id, 0)], assets=[
            (aid, 'offline' if state == 'available' else state)
            for aid, state in backup['availability']['assets']])
        assert current['availability'] == expected
        status_transition = 1
    assert current['epoch'] == max(changed['epoch'], backup['epoch']) + 1 + status_transition
    assert all(current[key] == backup[key] for key in ('assets', 'history', *RESTORE_STABLE_KEYS))


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
        assert current['availability'] == backup['availability']
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
        verify_restored_state(current, backup, changed, rollback)


def read_map_locations(db):
    return db.execute('''SELECT filename,round(gps_lat,2),round(gps_lon,2) FROM media_assets
        WHERE gps_lat IS NOT NULL OR gps_lon IS NOT NULL ORDER BY filename''').fetchall()


def verify_map(locations, observed):
    assert tuple(locations) == MAP_GPS
    assert observed == dict(mapRendered=True, mapErrorVisible=False, cells=[
        dict(lat=43.47, lon=11.88, count=2), dict(lat=43.5, lon=11.92, count=1)])


def frozen_pending_reason(db):
    if db.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone():
        return 'Background work is still active; observe the completed scan before submitting again'
    if not db.execute('SELECT 1 FROM selection_snapshots LIMIT 1').fetchone():
        return 'No frozen selection exists; capture the actual GUI order and freeze the selection before submitting again'
    return None


def checkpoint_observations(control, step):
    b.STATE['expectedCheckpoint'] = step
    b.event('awaitingNativeCheckpoint', step=step)
    request = control / 'step.json'
    deadline = time.monotonic() + 600
    hard_deadline = deadline + 1200
    frames = []
    high_water = 0
    while time.monotonic() < deadline:
        b.check()
        if not request.is_file():
            time.sleep(.25)
            continue
        try:
            observed = json.loads(request.read_text())
        except (UnicodeDecodeError, json.JSONDecodeError):
            observed = None
        finally:
            request.unlink()
        if not isinstance(observed, dict) or observed.pop('step', None) != step:
            b.event('nativeCheckpointNotReady', step=step, reason='Invalid request or unexpected checkpoint')
            continue
        if step in ('frozen', 'changed'):
            if observed == {'axReset': True}:
                frames.clear()
                b.event('nativePaginationCaptureReset', step=step)
                continue
            if set(observed) == {'axAppend'}:
                frame = observed['axAppend']
                added = frame if isinstance(frame, list) else [frame]
                try:
                    if (not added or len(frames) + len(added) > 64 or
                            any(not isinstance(item, str) or len(item.encode()) > 65536 for item in added)):
                        raise ValueError('Native AX segment count or size exceeds capture limits')
                    order, _, loaded, total = pagination_order(step, [*frames, *added])
                except ValueError as error:
                    b.event('nativeCheckpointNotReady', step=step, reason=str(error))
                else:
                    frames.extend(added)
                    if len(order) > high_water:
                        high_water = len(order)
                        deadline = min(hard_deadline, time.monotonic() + 600)
                    b.event('nativePaginationSegmentCaptured', step=step, segments=len(frames),
                            observedNames=len(order), loaded=loaded, total=total, firstName=order[0], lastName=order[-1])
                continue
            if set(observed) == {'axSelection'} and frames:
                observed['axFrames'] = list(frames)
        yield observed
    raise TimeoutError(f'No accepted native checkpoint: {step}')


AX_ROW = r'(?m)^[\t ]*\d+ '
AX_CARD = re.compile(AX_ROW + r'container (?:\([^\n)]*\) )?([^\n]+\.tiff)，[^\n，]+，[0-5] 星$')
AX_LOADED = re.compile(AX_ROW + r'text (\d+) 张已加载$')
AX_TOTAL = re.compile(AX_ROW + r'text · 当前查询 (\d+) 张$')
AX_FROZEN = re.compile(AX_ROW + r'text 已选 (\d+) 张 冻结快照$')
AX_EXCLUDED = re.compile(AX_ROW + r'text 已排除 (\d+) 张(?: 冻结于 [^\n]+)?$')
AX_SCOPE = re.compile(AX_ROW + r'text 筛选已改变；本次 (\d+) 张的冻结选区保持原范围。')
AX_DIRECTION = re.compile(AX_ROW + r'button 改为(升序|降序)$')
AX_LIBRARY = re.compile(AX_ROW + r'HTML content .*URL: tauri://localhost#/library$')
AX_SELECTION = re.compile(AX_ROW + r'text 已选 \d+ 张')
AX_EXPIRY_ERROR = re.compile(AX_ROW + r'text 冻结选区已过期或数量变化，请重新确认$')


def native_library_frame(frame):
    return (isinstance(frame, str) and frame.startswith('Window: ')
            and 'App: Travel Gallery Offline Bench.' in frame.split('\n', 1)[0]
            and AX_LIBRARY.search(frame) is not None)


def expiry_observation(step, evidence):
    frame = evidence.get('axFrame')
    if set(evidence) != {'axFrame'} or not native_library_frame(frame):
        raise ValueError('Supply original isolated native library AX rows for this checkpoint')
    frozen = step in ('expiry-frozen', 'expiry-reselected')
    rejected = step == 'expiry-rejected'
    total = str(len(MATCH) + len(OUTSIDE))
    if rejected:
        valid = AX_EXPIRY_ERROR.search(frame) and not AX_TOTAL.search(frame) and not AX_CARD.search(frame)
    else:
        valid = AX_TOTAL.findall(frame) == [total] and AX_CARD.search(frame) and not AX_EXPIRY_ERROR.search(frame)
    if not valid or len(AX_SELECTION.findall(frame)) != int(frozen) or AX_EXCLUDED.search(frame):
        raise ValueError('Native query, expiry notice, or selection does not match this checkpoint')
    if AX_FROZEN.findall(frame) != ([total] if frozen else []):
        raise ValueError('Native frozen count does not match this checkpoint')
    return dict(expiredNotice=rejected, frozenCount=int(total) if frozen else 0)


def verify_expiry_state(step, current, previous, at):
    assert all(current[key] == previous[key] for key in previous if key not in ('snapshots', 'members'))
    if step in ('expiry-rejected', 'expiry-refreshed'):
        assert current == previous
        return None
    previous_snapshots = set(previous['snapshots'])
    snapshots = set(current['snapshots'])
    assert previous_snapshots <= snapshots and len(snapshots) == len(previous_snapshots) + 1
    [added] = snapshots - previous_snapshots
    snapshot_id, filters, _, _, count, created, deadline = added
    asset_ids = {row[0] for row in current['assets']}
    assert {row[1] for row in current['assets']} == set(MATCH + OUTSIDE)
    assert count == len(asset_ids) and created <= at < deadline and deadline - created == SNAPSHOT_TTL_MS
    assert all(value is None for value in json.loads(filters).values())
    assert set(current['members']) == set(previous['members']) | {(snapshot_id, asset_id) for asset_id in asset_ids}
    return added


def set_controlled_expiry(database, pid, snapshot):
    # This connection can only open the catalog already locked by the owned QA process.
    assert database == b.DATA / 'catalog.sqlite' and not database.is_symlink()
    assert database.resolve(strict=True) == database
    assert picker.verified_process(database) == pid
    with closing(sqlite3.connect(database.as_uri() + '?mode=rw', uri=True)) as db, db:
        changed = db.execute('UPDATE selection_snapshots SET expires_at=0 WHERE id=? AND expires_at=?',
                             (snapshot[0], snapshot[-1]))
        assert changed.rowcount == 1


def one_ax_row(pattern, frame, label):
    values = pattern.findall(frame)
    if len(values) != 1:
        raise ValueError(f'Expected one native {label} row')
    return values[0]


def read_pagination_frame(step, frame):
    direction = '降序' if step == 'frozen' else '升序'
    if not native_library_frame(frame):
        raise ValueError('Expected a full isolated native library tree or its original AX rows')
    if one_ax_row(AX_DIRECTION, frame, 'sort direction') != direction:
        raise ValueError('Native sort direction changed during capture')
    cards = AX_CARD.findall(frame)
    if not cards or len(cards) != len(set(cards)):
        raise ValueError('Native frame has missing or duplicate cards')
    return cards, int(one_ax_row(AX_LOADED, frame, 'loaded count')), int(one_ax_row(AX_TOTAL, frame, 'query count'))


def pagination_order(step, frames):
    order, first_page, total = read_pagination_frame(step, frames[0])
    if not len(order) <= first_page < total:
        raise ValueError('First frame does not show a partial page')
    previous, loaded = order, first_page
    for frame in frames[1:]:
        cards, count, frame_total = read_pagination_frame(step, frame)
        if frame_total != total or not loaded <= count <= total:
            raise ValueError('Native loaded/query counts changed inconsistently')
        if cards[0] not in previous:
            raise ValueError('Adjacent native frames must overlap while scrolling forward')
        start = order.index(cards[0])
        overlap = min(len(order) - start, len(cards))
        if order[start:start + overlap] != cards[:overlap] or set(order).intersection(cards[overlap:]):
            raise ValueError('Native card order disagrees across frames')
        order = order + cards[overlap:]
        if len(order) > count:
            raise ValueError('Observed more cards than the native loaded count')
        previous, loaded = cards, count
    return order, first_page, loaded, total


def pagination_observation(step, evidence):
    frames = evidence.get('axFrames')
    selection = evidence.get('axSelection')
    if (set(evidence) != {'axFrames', 'axSelection'} or not isinstance(frames, list) or len(frames) < 2
            or not all(isinstance(frame, str) for frame in [*frames, selection])):
        raise ValueError('Supply ordered native pagination frames and a final selection frame')
    order, first_page, loaded, total = pagination_order(step, frames)
    if len(order) != total or loaded != total:
        raise ValueError('Native capture does not cover the entire paginated query')

    cards, loaded, selection_total = read_pagination_frame(step, selection)
    if (loaded, selection_total) != (total, total) or cards[0] not in order:
        raise ValueError('Selection frame does not show the completed query')
    start = order.index(cards[0])
    if order[start:start + len(cards)] != cards:
        raise ValueError('Selection frame disagrees with the observed card order')
    frozen = one_ax_row(AX_FROZEN, selection, 'frozen count')
    excluded = one_ax_row(AX_EXCLUDED, selection, 'excluded count')
    scopes = AX_SCOPE.findall(selection)
    if scopes not in ([], [frozen]) or (bool(scopes) != (step == 'changed')):
        raise ValueError('Native frozen scope notice is missing or inconsistent')
    return dict(order=order, firstPage=first_page, frozenCount=int(frozen), excludedCount=int(excluded),
                scopeNotice=bool(scopes))


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


def accept(candidate, fixture, release, env, *, check_restore=False, check_rejections=False, preflight=False):
    control = b.WORK / 'control'
    control.mkdir(exist_ok=preflight)
    source = (fixture / 'source').resolve()
    database = b.DATA / 'catalog.sqlite'
    backup_dir = b.WORK / 'backup'
    backup_probe = b.WORK / 'restore-version-probe.sqlite'
    if check_restore:
        backup_dir.mkdir()
        b.STATE['backupDirectory'] = str(backup_dir)
    b.STATE.update(status='ready', application=str(b.APP_PATH), expectedDatabase=str(database),
                   controlDirectory=str(control))
    b.event('readyForNativeGui', source=str(source), **b.check())
    pid, previous = None, None
    baseline, backup, changed = None, None, None
    expiry_state = None
    restore_steps = RESTORE_STEPS
    if check_rejections:
        restore_steps = (*RESTORE_STEPS[:-1], *RESTORE_VERSION_STEPS, RESTORE_STEPS[-1])
    steps = STEPS[:3] if preflight else STEPS[:-1] + ((*restore_steps, MAP_STEP) if check_restore else ())
    steps += EXPIRY_STEPS if check_rejections else ()
    for step in (*steps, 'finish'):
        for observed in checkpoint_observations(control, step):
            assert reuse.normal_state() == b.STATE['normalBefore']
            assert reuse.source_state() == b.STATE['sourceBefore']
            if step == 'finish':
                assert not observed and exact.exited(b.APP_PATH) and exact.exited(b.NORMAL)
                exact.check_release(release, b.NORMAL)
                if not preflight:
                    assert batch.verify_inputs() == b.STATE['sourceCommit']
                    exact.publish(candidate, release, env)
                b.STATE.pop('expectedCheckpoint')
                b.STATE.update(status='finished', qaQuitNormally=True)
                return
            current_pid = picker.verified_process(database)
            assert pid is None or current_pid == pid
            pid = current_pid
            with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
                db.execute('BEGIN')
                reason = frozen_pending_reason(db) if step == 'frozen' else None
                if reason is None:
                    state = read_catalog(db)
                    if step in (*restore_steps, MAP_STEP, *EXPIRY_STEPS) or (check_restore and step == 'time-restored'):
                        restore_state = read_restore_state(db)
                    if step == MAP_STEP:
                        locations = read_map_locations(db)
                    if step == 'backed-up':
                        completed_backups = db.execute("""SELECT status,progress_done,progress_total,failure_count
                            FROM jobs WHERE type='BACKUP'""").fetchall()
            if reason:
                b.event('nativeCheckpointNotReady', step=step, reason=reason)
                continue
            if step in ('frozen', 'changed'):
                try:
                    observed = pagination_observation(step, observed)
                except ValueError as error:
                    b.event('nativeCheckpointNotReady', step=step, reason=str(error))
                    continue
            if step in EXPIRY_STEPS:
                try:
                    observed = expiry_observation(step, observed)
                except ValueError as error:
                    b.event('nativeCheckpointNotReady', step=step, reason=str(error))
                    continue
            if step in RESTORE_VERSION_STEPS:
                try:
                    verify_backup_version_observation(step, observed, backup_probe)
                except AssertionError:
                    b.event('nativeCheckpointNotReady', step=step, reason='Expected native restore confirmation or version conflict')
                    continue
            break
        if step in RESTORE_VERSION_STEPS:
            assert restore_state == changed
            assert not list(b.DATA.glob('before-restore-*.sqlite'))
            assert not list(b.DATA.glob('restore-staging-*'))
            if step == 'backup-reviewing':
                assert read_backup(backup_probe) == backup
                rewrite_backup_version_probe(backup_probe)
            b.event('nativeBackupVersionCheckpoint', step=step, catalogEpoch=restore_state['epoch'])
            continue
        if step in EXPIRY_STEPS:
            added = verify_expiry_state(step, restore_state, expiry_state, time.time_ns() // 1_000_000)
            if added:
                observed['snapshotId'] = added[0]
            if step == 'expiry-frozen':
                set_controlled_expiry(database, pid, added)
                restore_state['snapshots'] = [(*row[:-1], 0) if row == added else row
                                              for row in restore_state['snapshots']]
            expiry_state = restore_state
            b.event('nativeSnapshotExpiryCheckpoint', step=step, controlledExpiry=True, **observed)
            continue
        if step in (*RESTORE_STEPS, MAP_STEP):
            [selected] = list(backup_dir.glob('travel-gallery-*.sqlite'))
            if step == 'backed-up':
                backup = read_backup(selected)
                assert completed_backups == [('completed', 1, 1, 0)]
            assert read_backup(selected) == backup
            rollback = None
            if step in ('backup-restored', MAP_STEP):
                [auto_backup] = list(b.DATA.glob('before-restore-*.sqlite'))
                rollback = read_backup(auto_backup)
                assert not list(b.DATA.glob('restore-staging-*'))
            if step == MAP_STEP:
                verify_restored_state(restore_state, backup, changed, rollback)
                verify_map(locations, observed)
                expiry_state = restore_state
            else:
                verify_restore(step, restore_state, observed, backup, baseline, changed, rollback)
            if step == 'rating-changed':
                changed = restore_state
                if check_rejections:
                    prepare_backup_version_probe(selected, backup_probe)
                    b.STATE['restoreVersionProbe'] = str(backup_probe)
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
    check_restore = approval['batch'].startswith('catalog-')
    candidate, fixture, release, env = build.build_candidate(
        approval, approval['batch'], 'CatalogEpoch' if check_restore else 'LibraryQuery',
        lambda source: write_sources(source, include_gps=check_restore), picker_only=True,
        after_regressions=measure_query_latency if check_restore else None)
    if check_restore:
        metadata = json.loads((batch.ROOT / 'src/assets/map/metadata.json').read_text())
        [font] = list((b.WORK / 'frontend-production/assets').glob('NotoSans-*.ttf'))
        assert exact.sha(font) == metadata['font']['files']['NotoSans.ttf']['sha256']
        b.event('candidateMapFontVerified', bytes=font.stat().st_size, sha256=exact.sha(font))
    preparation = json.loads((b.APP_PATH.parent / 'preparation.json').read_text())
    assert preparation['pickerOnly']
    assert not any('absolute-path' in name for name in preparation['parentEntitlements'])
    accept(candidate, fixture, release, env, check_restore=check_restore,
           check_rejections=is_deadline_batch(approval['batch']))


def is_deadline_batch(name):
    return name == DEADLINE_BATCH or name.startswith(DEADLINE_BATCH + '-')


def pagination_preflight():
    native_gui.require_unlocked_console()
    b.APPROVAL = batch.ROOT / 'docs/native-library-pagination-preflight-budget.json'
    b.RECEIPT = batch.ROOT / 'docs/native-library-pagination-preflight-latest.json'
    reuse.RUN_PREFIX = 'travel-gallery-library-pagination-preflight-'
    reuse.SCOPE = 'Reuse the accepted App in isolation to verify segmented native pagination; no build or publication'
    reuse.PLANNED_BYTES, reuse.PLANNED_FILES, reuse.SOURCE_FILES = 512 * 1024**2, 750, len(MATCH + OUTSIDE)
    reuse.SANDBOX_SEAL_ARGS = ('--webkit-mach-lookup', '--picker-only')
    reuse.write_sources = write_sources
    prepare = reuse.main
    release = json.loads((batch.ROOT / 'artifacts/build-info.json').read_text())
    exact.check_release(release, b.NORMAL)
    reuse.main = lambda: prepare(on_ready=lambda: accept(None, b.WORK / 'fixture', release, None, preflight=True))
    return reuse.run()


def register_standing(approval_path, batch_name):
    native_gui.require_unlocked_console()
    authority = batch.ROOT / 'AGENTS.md'
    scope = 'Full locked Rust, 50k synthetic query benchmark, one macOS candidate, and 130 owned TIFFs for native pagination, frozen review, capture time, backup/restore and map checks'
    if is_deadline_batch(batch_name):
        scope += ', plus a changed backup copy and controlled expiry of one isolated snapshot with explicit reselection'
    registration = dict(
        batch=batch_name, status='active', limits=batch.LIMITS,
        requiredFreeBytes=batch.LIMITS['logicalBytes'],
        authorizationBasis='AGENTS.md:standing-resource-2026-11-23',
        authorizationFileSha256=hashlib.sha256(authority.read_bytes()).hexdigest(),
        cleanupPolicy='end-of-batch',
        scope=scope + '; publish only after all checkpoints and normal QA exit',
        cleanup='Remove owned fixtures, catalogs, caches, builds, candidate and rollback copies on success, failure or interruption',
        recordedAt=datetime.now(timezone.utc).isoformat())
    with approval_path.open('x') as handle:
        json.dump(registration, handle, ensure_ascii=False, indent=2)
        handle.write('\n')


@contextmanager
def claim_approval(approval_path, batch_name):
    if approval_path.is_symlink():
        raise RuntimeError('Batch registration must not be a symbolic link')
    with approval_path.open('r+') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        approval = json.load(handle)
        if (approval.get('batch') != batch_name or approval.get('status') != 'active'
                or approval.get('claimedAt') or approval.get('closedAt')
                or approval.get('limits') != batch.LIMITS
                or approval.get('requiredFreeBytes') != batch.LIMITS['logicalBytes']):
            raise RuntimeError(f'This candidate needs a fresh, unclaimed {batch_name} registration')
        subprocess.run([*batch.BUDGET, '--approval-file', str(approval_path)],
                       cwd=batch.ROOT, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        native_gui.require_unlocked_console()
        approval['claimedAt'] = datetime.now(timezone.utc).isoformat()
        handle.seek(0)
        json.dump(approval, handle, ensure_ascii=False, indent=2)
        handle.truncate()
        handle.flush()
        yield approval


def main(arguments):
    if tuple(arguments) == ('--pagination-preflight',):
        return pagination_preflight()
    modes = {(): BATCH, ('--catalog-epoch',): EPOCH_BATCH, ('--catalog-epoch-ax',): EPOCH_AX_BATCH}
    standing = len(arguments) == 2 and arguments[0] == '--catalog-standing'
    if standing:
        batch_name = arguments[1]
        if not re.fullmatch(r'catalog-[a-z0-9][a-z0-9-]{0,47}', batch_name):
            raise ValueError('Use a fresh catalog- batch name with lowercase letters, digits and hyphens')
    elif tuple(arguments) in modes:
        batch_name = modes[tuple(arguments)]
    else:
        raise ValueError('Usage: native-library-query-build-batch.py [--pagination-preflight|--catalog-epoch|--catalog-epoch-ax|--catalog-standing catalog-NAME]')
    if batch_name != BATCH:
        b.APPROVAL = batch.ROOT / f'docs/native-{batch_name}-budget.json'
        b.RECEIPT = batch.ROOT / f'docs/native-{batch_name}-latest.json'
    if standing:
        if b.RECEIPT.exists() or b.RECEIPT.is_symlink():
            raise RuntimeError('Choose a fresh catalog- batch name to preserve the previous receipt')
        if not b.APPROVAL.exists() and not b.APPROVAL.is_symlink():
            register_standing(b.APPROVAL, batch_name)
    if not b.APPROVAL.is_file() or b.APPROVAL.is_symlink():
        raise RuntimeError(f'Fresh {batch_name} registration is required; no temporary files generated')
    batch.run = run
    with claim_approval(b.APPROVAL, batch_name) as approval:
        return batch.main(approval)


if __name__ == '__main__':
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from error
