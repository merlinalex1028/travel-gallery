"""Observe native navigation/focus and independently decode all screen caches."""
import argparse
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
import threading
import time
import zlib

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'picker', Path(__file__).with_name('native-picker-restart-batch.py'))
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-navigation-focus-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-navigation-focus-latest.json'
reuse.SCOPE = ('100 owned numbered PNGs: native viewer identity across repeated OS focus changes '
               'and independent screen-cache decode; no product compilation or physical-frame claim')
reuse.RUN_PREFIX = 'travel-gallery-native-navigation-focus-'
reuse.PLANNED_BYTES = 512 * 1024**2
reuse.PLANNED_FILES = 750
reuse.SOURCE_FILES = 100
STEPS = ('empty', 'imported', *(f'viewed-{count:03}' for count in range(10, 101, 10)))
FOCUS_START = 'imported'
REQUIRED_RETURNS = 20
WIDTH, HEIGHT = 1600, 1000
SCREEN_WIDTH, SCREEN_HEIGHT = 2560, 1600
DIGITS = (
    '111101101101111', '010110010010111', '111001111100111',
    '111001111001111', '101101111001001', '111100111001111',
    '111100111101111', '111001001001001', '111101111101111', '111101111001111',
)
baseline = None
completed = []
focus_stop = threading.Event()
focus_thread = None
base_main, base_save = reuse.main, b.save


def colour(index):
    return tuple(40 + index * step % 180 for step in (53, 97, 29))


def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)


def write_sources(source):
    for index in range(1, reuse.SOURCE_FILES + 1):
        pixels = bytearray(bytes(colour(index)) * (WIDTH * HEIGHT))
        for digit, character in enumerate(f'{index:03}'):
            for cell, filled in enumerate(DIGITS[int(character)]):
                if filled == '0':
                    continue
                x, y = 380 + digit * 300 + cell % 3 * 70, 300 + cell // 3 * 70
                for row in range(y, y + 60):
                    start = (row * WIDTH + x) * 3
                    pixels[start:start + 180] = b'\xff' * 180
        raw = b''.join(b'\0' + pixels[row * WIDTH * 3:(row + 1) * WIDTH * 3] for row in range(HEIGHT))
        png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', WIDTH, HEIGHT, 8, 2, 0, 0, 0))
        png += chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b'')
        (source / f'frame-{index:03}.png').write_bytes(png)


def appkit(expression):
    return subprocess.check_output(['/usr/bin/osascript', '-l', 'JavaScript', '-e',
        'ObjC.import("AppKit"); ' + expression], text=True, timeout=5).strip()


def await_value(probe, description):
    until = time.monotonic() + 8
    while time.monotonic() < until:
        if value := probe():
            return value
        time.sleep(.15)
    raise RuntimeError(description)


def focus_preflight():
    registry = subprocess.check_output(['/usr/sbin/ioreg', '-l', '-n', 'Root', '-d', '1'], text=True)
    assert '"IOConsoleLocked" = No' in registry, 'The console must be unlocked for focus observation'
    front = lambda: int(appkit('$.NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;'))
    other_pid = front()
    assert other_pid > 1
    launch = ['/usr/bin/open', '-a', str(b.APP_PATH)]
    subprocess.run(launch, check=True, timeout=5)
    database = b.DATA / 'catalog.sqlite'

    def live_qa():
        try:
            return picker.verified_process(database)
        except (RuntimeError, subprocess.CalledProcessError):
            return None

    pid = await_value(live_qa, 'Isolated QA and its catalog did not start')
    assert pid != other_pid
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert db.execute('SELECT count(*) FROM media_assets').fetchone() == (0,)
    b.STATE['isolatedEmptyCatalogVerified'] = True
    ns_app = '$.NSRunningApplication.runningApplicationWithProcessIdentifier'
    qa_asn = subprocess.check_output(['/usr/bin/lsappinfo', '-nonames', 'find', f'pid={pid}'],
                                     text=True, timeout=5).strip()
    assert qa_asn.startswith('ASN:')
    roles = b.STATE.setdefault('observedRoles', [])

    def confirm(expected_pid, role):
        await_value(lambda: front() == expected_pid, f'AppKit did not observe {role} frontmost')
        asn = subprocess.check_output(['/usr/bin/lsappinfo', '-nonames', 'front'], text=True, timeout=5).strip()
        assert asn.startswith('ASN:') and (asn == qa_asn) == (role == 'qa')
        roles.append(role)
        b.event('focusPreflight', role=role, appKitAndLaunchServicesAgree=True, **b.check())

    confirm(pid, 'qa')
    appkit(f'{ns_app}({other_pid}).activateWithOptions(2);')
    confirm(other_pid, 'other')
    subprocess.run(launch, check=True, timeout=5)
    confirm(pid, 'qa')
    assert live_qa() == pid
    assert appkit(f'{ns_app}({pid}).terminate;') == 'true', 'The QA declined to quit normally'
    await_value(lambda: subprocess.run(['/bin/ps', '-p', str(pid)], stdout=subprocess.DEVNULL).returncode != 0,
                'The QA did not quit normally')
    b.STATE.update(actualOSFocusRoundTrip=True, qaQuitNormally=True)


def observe_focus(asn):
    result = dict(samples=0, transitions=[], maximumSampleGapMs=0, errors=[])
    b.STATE['osFocusObservation'] = result
    previous_role, previous_time = None, None
    while not focus_stop.is_set():
        try:
            front = subprocess.check_output(['/usr/bin/lsappinfo', '-nonames', 'front'], text=True, timeout=3).strip()
            assert front.startswith('ASN:') and len(front) < 80
            now = time.monotonic()
            role = 'qa' if front == asn else 'other'
            result['samples'] += 1
            if previous_time is not None:
                result['maximumSampleGapMs'] = max(result['maximumSampleGapMs'], (now - previous_time) * 1000)
            if role != previous_role:
                result['transitions'].append(dict(atMonotonic=now, role=role))
            previous_time, previous_role = now, role
        except BaseException as error:
            result['errors'].append(str(error)[:200])
            return
        focus_stop.wait(.08)


def snapshot(label):
    global baseline, focus_thread
    assert label == STEPS[len(completed)]
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('BEGIN')
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not db.execute('PRAGMA foreign_key_check').fetchall()
        roots = db.execute('SELECT id,absolute_path,is_online FROM source_roots').fetchall()
        assets = db.execute('SELECT id,filename,width,height,file_state FROM media_assets ORDER BY filename,id').fetchall()
        reviews = db.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        assert not db.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
        screen = db.execute("""SELECT a.filename,t.cache_rel_path FROM thumbnail_cache t
            JOIN media_assets a ON a.id=t.asset_id WHERE t.variant='screen' ORDER BY a.filename""").fetchall()
        if label == 'empty':
            assert not roots and not assets and not reviews and not screen
            b.STATE['verifiedPid'] = pid
            asn = subprocess.check_output(['/usr/bin/lsappinfo', '-nonames', 'find', f'pid={pid}'], text=True).strip()
            assert asn.startswith('ASN:') and len(asn) < 80
            focus_thread = threading.Thread(target=observe_focus, args=(asn,), daemon=True)
            focus_thread.start()
        else:
            assert pid == b.STATE['verifiedPid']
            assert len(roots) == 1 and roots[0][1:] == (str((b.WORK / 'fixture/source').resolve()), 1)
            assert len(assets) == len(reviews) == reuse.SOURCE_FILES
            assert [row[1:] for row in assets] == [
                (f'frame-{index:03}.png', WIDTH, HEIGHT, 'available')
                for index in range(1, reuse.SOURCE_FILES + 1)]
            if label == 'imported':
                assert db.execute("""SELECT 1 FROM jobs WHERE type='SCAN' AND status='completed'
                    AND progress_done=? AND progress_total=? AND failure_count=0""",
                    (reuse.SOURCE_FILES, reuse.SOURCE_FILES)).fetchone()
                baseline = (roots, assets, reviews)
            else:
                assert (roots, assets, reviews) == baseline
                count = int(label.split('-')[1])
                assert {name for name, _ in screen} >= {f'frame-{i:03}.png' for i in range(1, count + 1)}
        assert reuse.source_state() == b.STATE['sourceBefore']
        assert reuse.normal_state() == b.STATE['normalBefore']
    if label == FOCUS_START:
        b.STATE['navigationStartedMonotonic'] = time.monotonic()
    if label == STEPS[-1]:
        b.STATE['navigationFinishedMonotonic'] = time.monotonic()
    completed.append(label)
    b.event('nativeNavigationCheckpoint', label=label, assets=len(assets),
            screenCaches=len(screen), sourceAndFormalUnchanged=True)


def check_screen_pixels(raw, index):
    name = f'frame-{index:03}.png'
    assert len(raw) >= 54 and raw[:2] == b'BM', f'{name}: invalid BMP header'
    offset, dib_size = struct.unpack_from('<II', raw, 10)
    width, height, planes, bits, compression = struct.unpack_from('<iiHHI', raw, 18)
    rows = abs(height)
    assert (width, rows) in {(WIDTH, HEIGHT), (SCREEN_WIDTH, SCREEN_HEIGHT)}, (
        f'{name}: unexpected screen dimensions {width}x{rows}')
    assert planes == 1 and compression == 0 and bits in (24, 32), f'{name}: unsupported BMP layout'
    stride = ((width * bits + 31) // 32) * 4
    assert dib_size >= 40 and 14 + dib_size <= offset <= len(raw) - rows * stride, f'{name}: truncated BMP'

    def matches(x, y, expected):
        x, y = x * width // WIDTH, y * rows // HEIGHT
        row = rows - 1 - y if height > 0 else y
        start = offset + row * stride + x * (bits // 8)
        rgb = raw[start:start + 3][::-1]
        return all(abs(actual - channel) <= 6 for actual, channel in zip(rgb, expected))

    base = colour(index)
    assert matches(20, 20, base), f'{name}: corner colour differs'
    for digit, character in enumerate(f'{index:03}'):
        for cell, filled in enumerate(DIGITS[int(character)]):
            x, y = 415 + digit * 300 + cell % 3 * 70, 330 + cell // 3 * 70
            expected = (255, 255, 255) if filled == '1' else base
            assert matches(x, y, expected), f'{name}: encoded digit {digit + 1}, cell {cell + 1} differs'
    return f'{width}x{rows}'


def decode_screen_caches():
    with closing(sqlite3.connect((b.DATA / 'catalog.sqlite').as_uri() + '?mode=ro', uri=True)) as db:
        rows = db.execute("""SELECT a.filename,t.cache_rel_path,t.source_byte_size,a.byte_size,
            t.source_mtime_ms,a.mtime_ms FROM thumbnail_cache t JOIN media_assets a ON a.id=t.asset_id
            WHERE t.variant='screen' ORDER BY a.filename""").fetchall()
    assert len(rows) == reuse.SOURCE_FILES, (
        f'Expected {reuse.SOURCE_FILES} screen cache references after exit, found {len(rows)}')
    decoded = b.STATE['independentScreenDecode'] = dict(count=0, dimensions={},
        decoder='macOS sips BMP', indexedVersionsMatched=False, colourTolerance=6,
        scope='Cache contents and per-image digit and colour identity; not proof of displayed DOM variant or physical frames')
    bmp = b.WORK / 'decoded-screen.bmp'
    for index, (name, relative, cache_size, size, cache_mtime, mtime) in enumerate(rows, 1):
        assert name == f'frame-{index:03}.png', f'Unexpected asset at index {index}: {name}'
        assert cache_size == size and cache_mtime == mtime, f'{name}: cache source version differs'
        cache = b.CACHE / 'thumbs'
        path = cache / relative
        assert path.is_file() and not path.is_symlink() and path.resolve().is_relative_to(cache.resolve()), (
            f'{name}: screen cache file is missing or outside the isolated cache')
        try:
            result = subprocess.run(['/usr/bin/sips', '-s', 'format', 'bmp', str(path), '--out', str(bmp)],
                                    capture_output=True, text=True, timeout=15)
            assert result.returncode == 0 and bmp.is_file(), f'{name}: sips failed: {result.stderr.strip()[:200]}'
            dimensions = check_screen_pixels(bmp.read_bytes(), index)
            decoded['count'] += 1
            decoded['dimensions'][dimensions] = decoded['dimensions'].get(dimensions, 0) + 1
            b.check()
        finally:
            bmp.unlink(missing_ok=True)
    decoded['indexedVersionsMatched'] = True


def main():
    try:
        base_main()
        assert tuple(completed) == STEPS, f'Missing database checkpoints: {STEPS[len(completed):]}'
        executable = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve(strict=True))
        processes = subprocess.check_output(['ps', '-axww', '-o', 'comm='], text=True).splitlines()
        assert executable not in {line.strip() for line in processes}, 'Quit QA before finish'
        b.STATE['qaQuitNormally'] = True
    finally:
        focus_stop.set()
        if focus_thread:
            focus_thread.join(timeout=4)
    focus = b.STATE['osFocusObservation']
    roles = [item['role'] for item in focus['transitions']
             if b.STATE['navigationStartedMonotonic'] <= item['atMonotonic'] <= b.STATE['navigationFinishedMonotonic']]
    returns = sum(a == 'other' and c == 'qa' for a, c in zip(roles, roles[1:]))
    focus['returnsDuringNavigation'] = returns
    decode_screen_caches()
    assert not focus['errors'] and focus['samples'] > 0, 'OS focus sampler did not complete'
    assert returns >= REQUIRED_RETURNS, f'Observed {returns} OS focus returns; expected at least {REQUIRED_RETURNS}'


def save():
    if b.STATE.get('temporaryDirectoryRemoved') and b.WORK:
        for key in ('temporaryRoot', 'application', 'expectedDatabase', 'verifiedPid'):
            b.STATE.pop(key, None)
        for root in {str(b.WORK), str(b.WORK.resolve())}:
            b.STATE.update(json.loads(json.dumps(b.STATE).replace(root, '<cleaned run>')))
    base_save()


reuse.write_sources, reuse.snapshot, reuse.main, b.save = write_sources, snapshot, main, save

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--focus-preflight', action='store_true', help='Verify OS focus in an empty isolated catalog')
    mode.add_argument('--screen-smoke', action='store_true', help='Verify three actual screen caches and one focus return')
    args = parser.parse_args()
    if args.focus_preflight:
        b.APPROVAL = reuse.ROOT / 'docs/native-navigation-focus-preflight-budget.json'
        b.RECEIPT = reuse.ROOT / 'docs/native-navigation-focus-preflight.json'
        reuse.SCOPE = 'One owned but unimported PNG; verify actual macOS focus round trip with an empty isolated catalog'
        reuse.RUN_PREFIX = 'travel-gallery-native-navigation-focus-preflight-'
        reuse.SOURCE_FILES = 1
        reuse.main = lambda: base_main(on_ready=focus_preflight)
    elif args.screen_smoke:
        b.APPROVAL = reuse.ROOT / 'docs/native-navigation-screen-smoke-budget.json'
        b.RECEIPT = reuse.ROOT / 'docs/native-navigation-screen-smoke.json'
        reuse.SCOPE = 'Three owned PNGs; decode actual screen caches and verify one OS focus return during viewing'
        reuse.RUN_PREFIX = 'travel-gallery-native-navigation-screen-smoke-'
        reuse.SOURCE_FILES = 3
        STEPS = ('empty', 'imported', 'viewed-001', 'viewed-003')
        FOCUS_START = 'viewed-001'
        REQUIRED_RETURNS = 1
    timer = threading.Timer(50 if args.focus_preflight else 600 if args.screen_smoke else 1200,
                            lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
