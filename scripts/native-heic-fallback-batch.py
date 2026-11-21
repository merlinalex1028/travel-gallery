"""Check HEIF preview fallback in an isolated copy of the accepted macOS release."""
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
spec = importlib.util.spec_from_file_location(
    'picker', Path(__file__).with_name('native-picker-restart-batch.py'))
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-heic-fallback-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-heic-fallback-latest.json'
reuse.SCOPE = 'QA-12: decodable HEVC image and software-decodable 12-bit HEVC image that macOS sips cannot convert'
reuse.RUN_PREFIX = 'travel-gallery-native-heic-fallback-'
reuse.SOURCE_FILES = 2
reuse.PLANNED_FILES = 300
reuse.PLANNED_BYTES = 512 * 1024**2
FFMPEG = '/opt/homebrew/bin/ffmpeg'
FFPROBE = '/opt/homebrew/bin/ffprobe'
SIPS = '/usr/bin/sips'
CASES = (
    ('01-main.heic', 8, 'yuv420p', 'mainstillpicture', True),
    ('02-rext.heic', 12, 'yuv444p12le', 'main444-12-intra', False),
)
STEPS = ('empty', 'imported', 'viewed')
completed = []
base_main, base_save = reuse.main, b.save


def pack(fmt, *values):
    return struct.pack('>' + fmt, *values)


def box(kind, data):
    return pack('I4s', 8 + len(data), kind) + data


def full_box(kind, version, flags, data):
    return box(kind, pack('I', version << 24 | flags) + data)


def payload(parent, target):
    at = 0
    while at < len(parent):
        size, kind = struct.unpack_from('>I4s', parent, at)
        assert 8 <= size <= len(parent) - at, (target, kind, size)
        if kind == target:
            return parent[at + 8:at + size]
        at += size
    raise ValueError(f'Missing {target!r} in generated MP4')


def heif(mp4, depth):
    # Reuse the encoder's configuration and one length-prefixed HEVC sample.
    coded = payload(mp4, b'mdat')
    track = payload(payload(mp4, b'moov'), b'trak')
    table = payload(payload(payload(track, b'mdia'), b'minf'), b'stbl')
    sample_entry = payload(payload(table, b'stsd')[8:], b'hvc1')
    config = payload(sample_entry[78:], b'hvcC')
    brand = b'heic' if depth == 8 else b'mif1'
    ftyp = box(b'ftyp', brand + pack('I', 0) + b'mif1' + (b'heic' if depth == 8 else b''))
    handler = full_box(b'hdlr', 0, 0, pack('I4sIII', 0, b'pict', 0, 0, 0) + b'QA\0')
    primary = full_box(b'pitm', 0, 0, pack('H', 1))
    item = full_box(b'infe', 2, 0, pack('HH4s', 1, 0, b'hvc1') + b'QA\0')
    items = full_box(b'iinf', 0, 0, pack('H', 1) + item)
    properties = box(b'ipco', full_box(b'ispe', 0, 0, pack('II', 64, 48))
                     + full_box(b'pixi', 0, 0, bytes([3, depth, depth, depth])) + box(b'hvcC', config))
    associations = full_box(b'ipma', 0, 0, pack('IHB', 1, 1, 3) + bytes([1, 2, 0x83]))

    def meta(offset):
        location = full_box(b'iloc', 0, 0, bytes([0x44, 0]) + pack('HHHHII', 1, 1, 0, 1, offset, len(coded)))
        return full_box(b'meta', 0, 0, handler + primary + location + items + box(b'iprp', properties + associations))

    return ftyp + meta(len(ftyp) + len(meta(0)) + 8) + box(b'mdat', coded)


def write_sources(source):
    results = []
    scratch = source.parent / 'verify'
    scratch.mkdir()
    env = dict(os.environ, TMPDIR=str(scratch))
    for name, depth, pixel_format, profile, native_ok in CASES:
        mp4, jpeg = scratch / 'frame.mp4', scratch / 'frame.jpg'
        path = source / name
        subprocess.run([FFMPEG, '-v', 'error', '-nostdin', '-f', 'lavfi', '-i', 'testsrc2=size=64x48:rate=1',
                        '-frames:v', '1', '-pix_fmt', pixel_format, '-c:v', 'libx265', '-profile:v', profile,
                        '-x265-params', 'log-level=error:pools=1:frame-threads=1:keyint=1', '-tag:v', 'hvc1', str(mp4)],
                       check=True, timeout=15)
        path.write_bytes(heif(mp4.read_bytes(), depth))
        format_data = json.loads(subprocess.check_output([FFPROBE, '-v', 'error', '-show_entries',
            'stream=codec_name,pix_fmt,width,height', '-of', 'json', str(path)], text=True, timeout=10))
        assert format_data['streams'] == [dict(codec_name='hevc', width=64, height=48, pix_fmt=pixel_format)]
        frame = subprocess.check_output([FFMPEG, '-v', 'error', '-xerror', '-i', str(path), '-frames:v', '1',
                                         '-f', 'framemd5', '-'], text=True, timeout=10)
        [checksum] = [row.rsplit(',', 1)[-1].strip() for row in frame.splitlines() if row and not row.startswith('#')]
        result = subprocess.run([SIPS, '-s', 'format', 'jpeg', str(path), '--out', str(jpeg)],
                                env=env, capture_output=True, timeout=10)
        assert (result.returncode == 0 and jpeg.is_file()) == native_ok, (name, result.returncode, result.stderr)
        results.append(dict(name=name, pixelFormat=pixel_format, softwareFrameMD5=checksum, sipsExit=result.returncode,
                            jpegWritten=jpeg.is_file()))
        mp4.unlink()
        jpeg.unlink(missing_ok=True)
    b.STATE['fixtureDecode'] = results


def snapshot(label):
    assert len(completed) < len(STEPS) and label == STEPS[len(completed)]
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('BEGIN')
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not db.execute('PRAGMA foreign_key_check').fetchone()
        roots = db.execute('SELECT absolute_path,is_online FROM source_roots').fetchall()
        assets = db.execute('''SELECT filename,id,file_state,extension,width,height,byte_size
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
        assert len(assets) == len(reviews) == len(CASES) == 2
        expected = [('available', 'heic', 64, 48), ('unsupported', 'heic', None, None)]
        for row, (name, *_), fields in zip(assets, CASES, expected):
            assert row[0] == name and row[2:-1] == fields and row[-1] == (source / name).stat().st_size, row
        assert scan and set(scan) == {('completed', 2, 2, 0)}, scan
        assert cache and {name for name, _, _ in cache} == {CASES[0][0]}, cache
        if label == 'imported':
            b.STATE['catalogIdentity'] = json.dumps([assets, reviews])
        else:
            assert b.STATE['catalogIdentity'] == json.dumps([assets, reviews])
            [relative] = [name for _, variant, name in cache if variant == 'screen']
            assert relative == Path(relative).name
            screen = b.CACHE / 'thumbs' / relative
            assert screen.is_file() and not screen.is_symlink()
    assert reuse.source_state() == b.STATE['sourceBefore']
    assert reuse.normal_state() == b.STATE['normalBefore']
    completed.append(label)
    b.event('nativeHeicCheckpoint', label=label, media=[row[:1] + row[2:] for row in assets], scans=scan,
            caches=[(name, variant) for name, variant, _ in cache], sourceAndFormalUnchanged=True)


def main():
    base_main()
    assert tuple(completed) == STEPS, f'Missing HEIC checkpoints: {STEPS[len(completed):]}'
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
    timer = Timer(600, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
