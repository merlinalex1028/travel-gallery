"""Verify actual RAW and MOV fallback in the currently delivered macOS app."""
from contextlib import closing
import importlib.util
import hashlib
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
spec = importlib.util.spec_from_file_location(
    'picker', Path(__file__).with_name('native-picker-restart-batch.py'))
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-format-fallback-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-format-fallback-latest.json'
reuse.SCOPE = 'QA-12: synthetic uncompressed DNG without JPEG, actual FFV1/MOV and H.264/MOV; no build'
reuse.RUN_PREFIX = 'travel-gallery-native-format-fallback-'
reuse.SOURCE_FILES = 3
reuse.PLANNED_FILES = 300
reuse.PLANNED_BYTES = 512 * 1024**2
RAW, UNSUPPORTED, CONTROL = '01-no-jpeg.dng', '02-ffv1.mov', '03-h264.mov'
STEPS = ('empty', 'imported', 'viewed')
base_main, base_save = reuse.main, b.save
completed = []


def raw_mosaic():
    # Classic TIFF with an uncompressed 16-bit Bayer strip and DNG tags. No JPEG IFD.
    u16 = lambda *values: struct.pack('<' + 'H' * len(values), *values)
    u32 = lambda *values: struct.pack('<' + 'I' * len(values), *values)
    width, height = 64, 48
    pixels = u16(*(256 + (x * 67 + y * 36) % 3072 for y in range(height) for x in range(width)))
    tags = {
        254: (4, 1, u32(0)), 256: (4, 1, u32(width)), 257: (4, 1, u32(height)),
        258: (3, 1, u16(16)), 259: (3, 1, u16(1)), 262: (3, 1, u16(32803)),
        271: (2, 10, b'Travel QA\0'), 272: (2, 11, b'Raw Mosaic\0'),
        274: (3, 1, u16(1)), 277: (3, 1, u16(1)), 278: (4, 1, u32(height)),
        279: (4, 1, u32(len(pixels))), 284: (3, 1, u16(1)),
        33421: (3, 2, u16(2, 2)), 33422: (1, 4, b'\x00\x01\x01\x02'),
        50706: (1, 4, b'\x01\x04\x00\x00'), 50707: (1, 4, b'\x01\x01\x00\x00'),
        50708: (2, 14, b'QA Raw Mosaic\0'), 50710: (1, 3, b'\x00\x01\x02'),
        50711: (3, 1, u16(1)), 50714: (3, 1, u16(0)), 50717: (4, 1, u32(4095)),
        50721: (10, 9, struct.pack('<18i', *[part for value in (1,0,0,0,1,0,0,0,1) for part in (value,1)])),
        50778: (3, 1, u16(21)),
    }
    tags[273] = (4, 1, u32(8 + 2 + 12 * (len(tags) + 1) + 4))
    data = bytearray(pixels)
    entries = []
    for tag, (kind, count, value) in sorted(tags.items()):
        if len(value) > 4:
            offset = 8 + 2 + 12 * len(tags) + 4 + len(data)
            data.extend(value)
            data.extend(b'\0' * (len(data) % 2))
            value = u32(offset)
        entries.append(u16(tag, kind) + u32(count) + value.ljust(4, b'\0'))
    return b'II*\0' + u32(8) + u16(len(tags)) + b''.join(entries) + u32(0) + data


def write_sources(source):
    encoder = Path('/opt/homebrew/bin/ffmpeg')
    bundle = b.NORMAL / 'Contents/MacOS'
    release = json.loads((reuse.ROOT / 'artifacts/build-info.json').read_text())
    for tool in ('ffmpeg', 'ffprobe'):
        with (bundle / tool).open('rb') as stream:
            assert hashlib.file_digest(stream, 'sha256').hexdigest() == release[f'{tool}Sha256']
    data = raw_mosaic()
    assert b'\xff\xd8' not in data
    (source / RAW).write_bytes(data)
    metadata = {}
    for name, encoder_id, codec in ((UNSUPPORTED, 'ffv1', 'ffv1'), (CONTROL, 'libx264', 'h264')):
        path = source / name
        subprocess.run([str(encoder), '-v', 'error', '-nostdin', '-f', 'lavfi', '-i',
            'testsrc2=size=160x120:rate=12:duration=2', '-c:v', encoder_id, '-an', '-f', 'mov', str(path)], check=True, timeout=20)
        raw = subprocess.check_output([str(bundle / 'ffprobe'), '-v', 'error', '-show_streams', '-show_format',
            '-of', 'json', str(path)], text=True, timeout=15)
        probe = json.loads(raw)
        [video] = probe['streams']
        assert probe['format']['tags']['major_brand'].strip() == 'qt'
        assert (video['codec_name'], video['width'], video['height'], float(probe['format']['duration'])) == (codec, 160, 120, 2.)
        metadata[name] = dict(codec=codec, width=160, height=120, durationMs=2000, container='QuickTime')
    b.STATE['fixtureVideoProbe'] = metadata


def snapshot(label):
    assert len(completed) < len(STEPS) and label == STEPS[len(completed)]
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('BEGIN')
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not db.execute('PRAGMA foreign_key_check').fetchone()
        roots = db.execute('SELECT absolute_path,is_online FROM source_roots').fetchall()
        assets = db.execute('''SELECT filename,id,file_state,extension,width,height,duration_ms,codec,byte_size
            FROM media_assets ORDER BY filename''').fetchall()
        reviews = db.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        scan = db.execute("SELECT status,progress_done,progress_total,failure_count FROM jobs WHERE type='SCAN'").fetchall()
        cache = db.execute('''SELECT a.filename,t.variant,t.cache_rel_path FROM thumbnail_cache t
            JOIN media_assets a ON a.id=t.asset_id ORDER BY 1,2''').fetchall()
        assert not db.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
    if label == 'empty':
        assert not (roots or assets or reviews or scan or cache)
        b.STATE['verifiedPid'] = pid
    else:
        assert pid == b.STATE['verifiedPid']
        source = (b.WORK / 'fixture/source').resolve()
        assert roots == [(str(source), 1)]
        assert len(assets) == len(reviews) == 3
        expected = [('unsupported', 'dng', 64, 48, None, None),
                    ('available', 'mov', 160, 120, 2000, 'ffv1'),
                    ('available', 'mov', 160, 120, 2000, 'h264')]
        for row, name, fields in zip(assets, (RAW, UNSUPPORTED, CONTROL), expected):
            assert row[0] == name and row[2:-1] == fields and row[-1] == (source / name).stat().st_size, row
        assert scan and set(scan) == {('completed', 3, 3, 0)}, scan
        if label == 'imported':
            b.STATE['catalogIdentity'] = json.dumps([assets, reviews])
        else:
            assert b.STATE['catalogIdentity'] == json.dumps([assets, reviews])
            screen = {name: relative for name, variant, relative in cache if variant == 'screen'}
            assert set(screen) == {UNSUPPORTED, CONTROL}, cache
            assert all(name != RAW for name, _, _ in cache)
            cache_root = (b.CACHE / 'thumbs').resolve(strict=True)
            for relative in screen.values():
                assert relative == Path(relative).name and relative.endswith('.jpg')
                path = cache_root / relative
                assert not path.is_symlink() and path.is_file()
                decoded = subprocess.check_output(['/usr/bin/sips', '-g', 'pixelWidth', '-g', 'pixelHeight', str(path)],
                                                  text=True, timeout=10)
                assert 'pixelWidth: 2560' in decoded and 'pixelHeight: 1920' in decoded, decoded
    assert reuse.source_state() == b.STATE['sourceBefore']
    assert reuse.normal_state() == b.STATE['normalBefore']
    completed.append(label)
    b.event('nativeFormatCheckpoint', label=label, media=[row[:1] + row[2:] for row in assets],
            scans=scan, caches=[(name, variant) for name, variant, _ in cache], sourceAndFormalUnchanged=True)


def main():
    base_main()
    assert tuple(completed) == STEPS, f'Missing format checkpoints: {STEPS[len(completed):]}'
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
