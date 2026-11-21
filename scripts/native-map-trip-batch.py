"""Verify an isolated release's geographic drilldown against owned EXIF fixtures."""
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import signal
import sqlite3
import struct
import subprocess
import sys
from threading import Timer

sys.dont_write_bytecode = True


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


sorting = load('native-sorting-batch')
picker = load('native-picker-restart-batch')
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-map-trip-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-map-trip-latest.json'
reuse.SCOPE = 'PG-15: four EXIF TIFFs, trip GPS scope, trail and gallery drilldown on current macOS release'
reuse.RUN_PREFIX = 'travel-gallery-native-map-trip-'
reuse.SOURCE_FILES = 4
reuse.PLANNED_FILES = 300
reuse.PLANNED_BYTES = 512 * 1024**2
TRIP = 'Map QA Trip'
CASES = (
    ('01-trip-east.tiff', (43, 28, 12), (11, 52, 48)),
    ('02-trip-west.tiff', (43, 28, 12), (11, 43, 12)),
    ('03-outside-same-cell.tiff', (43, 28, 12), (11, 52, 48)),
    ('04-no-gps.tiff', None, None),
)
STEPS = ('empty', 'imported', 'assigned', 'drilldown')
completed = []
base_main, base_save = reuse.main, b.save


def add_gps(tiff, latitude, longitude):
    data = bytearray(tiff)
    data.extend(b'\0' * (len(data) % 2))
    entry_count, = struct.unpack_from('<H', data, 8)
    entries = [bytes(data[p:p + 12]) for p in range(10, 10 + 12 * entry_count, 12)]
    # Append a replacement root IFD, leaving the existing strip and EXIF offsets intact.
    root = len(data)
    gps = root + 2 + 12 * (entry_count + 1) + 4
    rational_start = gps + 2 + 4 * 12 + 4
    entries.append(struct.pack('<HHII', 34853, 4, 1, gps))
    entries.sort(key=lambda row: struct.unpack_from('<H', row)[0])
    struct.pack_into('<I', data, 4, root)
    data.extend(struct.pack('<H', len(entries)) + b''.join(entries) + bytes(4))
    data.extend(struct.pack('<H', 4))
    for tag, kind, count, value in ((1, 2, 2, ord('N')), (2, 5, 3, rational_start),
                                     (3, 2, 2, ord('E')), (4, 5, 3, rational_start + 24)):
        data.extend(struct.pack('<HHII', tag, kind, count, value))
    data.extend(bytes(4))
    for value in (*latitude, *longitude):
        data.extend(struct.pack('<II', value, 1))
    return data


def write_sources(source):
    for index, (name, lat, lon) in enumerate(CASES):
        content = sorting.tiny_tiff(index, f'2024:04:10 08:0{index}:00', '+02:00')
        (source / name).write_bytes(add_gps(content, lat, lon) if lat else content)


def snapshot(label):
    assert len(completed) < len(STEPS) and label == STEPS[len(completed)]
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('BEGIN')
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not db.execute('PRAGMA foreign_key_check').fetchone()
        assert not db.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
        roots = db.execute('SELECT absolute_path,is_online FROM source_roots').fetchall()
        photos = db.execute('''SELECT filename,id,file_state,round(gps_lat,2),round(gps_lon,2),
            capture_local,capture_tz_min,capture_time_source FROM media_assets ORDER BY filename''').fetchall()
        reviews = db.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        trips = db.execute('SELECT id,name FROM collections').fetchall()
        members = db.execute('SELECT collection_id, asset_id FROM collection_assets ORDER BY 1,2').fetchall()
        queries = db.execute('SELECT request_json FROM query_contexts').fetchall()
    if label == 'empty':
        assert not (roots or photos or reviews or trips or members)
        b.STATE['verifiedPid'] = pid
    else:
        assert pid == b.STATE['verifiedPid']
        assert roots == [(str((b.WORK / 'fixture/source').resolve()), 1)]
        assert len(photos) == len(reviews) == 4
        cells = [(43.47, 11.88), (43.47, 11.72), (43.47, 11.88), (None, None)]
        for i, (row, (name, *_), (lat, lon)) in enumerate(zip(photos, CASES, cells)):
            assert row[0] == name and row[2:] == ('available', lat, lon, f'2024-04-10 08:0{i}:00', 120, 'exif'), row
        if label == 'imported':
            assert not (trips or members)
            b.STATE['catalogIdentity'] = json.dumps([photos, reviews])
        else:
            assert b.STATE['catalogIdentity'] == json.dumps([photos, reviews])
            [(trip_id, name)] = trips
            assert name == TRIP
            assert set(members) == {(trip_id, photos[0][1]), (trip_id, photos[1][1]), (trip_id, photos[3][1])}
            if label == 'drilldown':
                filters = [json.loads(row[0])['filter'] for row in queries]
                assert any(f['collectionId'] == trip_id and f['gpsCell'] == [43.47, 11.88] for f in filters)
    assert reuse.source_state() == b.STATE['sourceBefore']
    assert reuse.normal_state() == b.STATE['normalBefore']
    completed.append(label)
    b.event('nativeMapTripCheckpoint', label=label, assets=[row[:1] + row[2:] for row in photos],
            tripMembers=len(members), sourceAndFormalUnchanged=True)


def main():
    base_main()
    assert tuple(completed) == STEPS, f'Missing map checkpoints: {STEPS[len(completed):]}'
    executable = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve())
    commands = subprocess.check_output(['ps', '-axww', '-o', 'comm='], text=True).splitlines()
    assert executable not in {command.strip() for command in commands}, 'Quit QA normally before finish'
    b.STATE['qaQuitNormally'] = True


def save():
    if b.STATE.get('temporaryDirectoryRemoved') and b.WORK:
        for key in ('temporaryRoot', 'application', 'expectedDatabase', 'verifiedPid', 'catalogIdentity'):
            b.STATE.pop(key, None)
        for root in {str(b.WORK), str(b.WORK.resolve())}:
            b.STATE.update(json.loads(json.dumps(b.STATE).replace(root, '<cleaned run>')))
    base_save()


reuse.write_sources, reuse.snapshot, reuse.main, b.save = write_sources, snapshot, main, save

if __name__ == '__main__':
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
