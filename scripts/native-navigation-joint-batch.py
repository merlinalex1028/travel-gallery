"""One bounded QA observer build; correlate real navigation, WebView and OS focus."""
from contextlib import closing
from datetime import datetime, timezone
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


focus = load('native-navigation-focus-batch')
analysis = load('analyze-native-navigation')
reuse, b = focus.reuse, focus.b
b.DATA = b.LIBRARY / 'Application Support' / b.IDENTIFIER
b.CACHE = b.LIBRARY / 'Caches' / b.IDENTIFIER
b.APPROVAL = ROOT / 'docs/native-navigation-joint-budget.json'
b.RECEIPT = ROOT / 'docs/native-navigation-joint-latest.json'
LIMITS = dict(files=25000, logicalBytes=4 * 1024**3, sourceFiles=100)


require_unlocked_console = load('native-gui-preflight').require_unlocked_console
pixels = load('native_navigation_pixels')


def front_asn(*args):
    return subprocess.check_output(['/usr/bin/lsappinfo', '-nonames', *args], text=True, timeout=3).strip()


def observe_window_pixels(index):
    try:
        require_unlocked_console()
    except RuntimeError as error:
        raise ValueError(str(error)) from error
    assert type(index) is int and 1 <= index <= 100
    database = b.DATA / 'catalog.sqlite'
    pid = focus.picker.verified_process(database)
    assert pid == b.STATE['verifiedPid'] and len(b.STATE['manifest']) == 100

    asn = front_asn('find', f'pid={pid}')
    if not asn.startswith('ASN:') or front_asn('front') != asn:
        raise ValueError('The isolated viewer must be in front')
    windows = json.loads(focus.appkit(f'''
        ObjC.import('CoreGraphics');
        const windows = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(17, 0)));
        JSON.stringify(windows.filter(w => w.kCGWindowOwnerPID === {pid} && w.kCGWindowLayer === 0)
                              .map(w => [w.kCGWindowNumber, w.kCGWindowBounds.Width,
                                         w.kCGWindowBounds.Height]));
    '''))
    if len(windows) != 1:
        raise ValueError('Expected one visible window owned by the isolated process')
    window_id, width, height = windows[0]
    if type(window_id) is not int or window_id <= 0 or not (0 < width <= 4096 and 0 < height <= 4096):
        raise ValueError('Invalid window ID or size')

    with tempfile.TemporaryDirectory(prefix='window-pixels-', dir=b.WORK) as owned:
        image, bitmap = Path(owned) / 'window.png', Path(owned) / 'window.bmp'
        started = time.time_ns() / 1_000_000
        subprocess.run(['/usr/sbin/screencapture', '-x', '-o', '-a', '-l', str(window_id), str(image)],
                       check=True, capture_output=True, timeout=8)
        finished = time.time_ns() / 1_000_000
        assert front_asn('front') == asn and focus.picker.verified_process(database) == pid
        b.check()
        subprocess.run(['/usr/bin/sips', '-m', '/System/Library/ColorSync/Profiles/sRGB Profile.icc',
                        '-s', 'format', 'bmp', str(image), '--out', str(bitmap)],
                       check=True, capture_output=True, timeout=8)
        b.check()
        stats = pixels.check_window_pixels(bitmap.read_bytes(), focus.WIDTH, focus.HEIGHT, focus.colour(index),
                                           [focus.DIGITS[int(value)] for value in f'{index:03}'])
        with image.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        return dict(index=index, filename=f'frame-{index:03}.png', pid=pid, windowId=window_id,
                    startedUnixMs=started, finishedUnixMs=finished, imageSha256=digest, **stats)


def read_probe():
    [log] = list((b.DATA / 'performance').glob('*.jsonl'))
    # Ignore a partially appended final line while the live observer is flushing.
    raw = log.read_bytes().rsplit(b'\n', 1)[0] + b'\n'
    rows = [json.loads(line) for line in raw.splitlines()]
    [start] = [row for row in rows if row['type'] == 'nativeStart']
    assert start['pid'] == b.STATE['verifiedPid'] and start['identifier'] == b.IDENTIFIER
    events = [row['event'] for row in rows if row['type'] == 'frontendBatchEvent']
    return raw, start, events


def visible_clock(start, events, now):
    clock = next((event for event in reversed(events) if event['type'] == 'clock'), None)
    if clock is None or clock['runId'] != start['runId']:
        return False
    started = datetime.fromisoformat(start['atUtc']).timestamp() * 1000 - start['nativeElapsedMs']
    age = now - (started + clock['nativeRespondedMs'])
    return 0 <= age <= 5000 and clock['visibility'] == 'visible' and clock['focused'] is True


def confirm_foreground():
    require_unlocked_console()
    database = b.DATA / 'catalog.sqlite'
    pid = focus.picker.verified_process(database)
    assert pid == b.STATE['verifiedPid'], 'The isolated viewer process changed'
    asn = front_asn('find', f'pid={pid}')
    if not asn.startswith('ASN:') or front_asn('front') != asn:
        raise ValueError('The isolated viewer must be in front')
    _, start, events = read_probe()
    if not visible_clock(start, events, time.time_ns() / 1_000_000):
        raise ValueError('The same WebView has no fresh visible and focused clock sample')
    if front_asn('front') != asn or focus.picker.verified_process(database) != pid:
        raise ValueError('The isolated viewer lost foreground identity during the check')
    return pid


def probe_result(count):
    raw, start, events = read_probe()
    result = analysis.analyze_events(events, b.STATE['manifest'],
                                    [row['filename'] for row in b.STATE['order']],
                                    first_position=2, end_id=count,
                                    expected_focus_photos=count, required_focus_returns=1 if count == 3 else 20)
    result['focusVariant']['osCorroboration'] = analysis.corroborate_os_focus(
        result['focusVariant'], b.STATE['osFocusObservation'], start, events)
    result.update(rawSha256=hashlib.sha256(raw).hexdigest(), rawBytes=len(raw), run=start)
    return result


def run_gui():
    control = b.WORK / 'control'
    control.mkdir()
    b.STATE.update(status='ready', application=str(b.APP_PATH),
                   expectedDatabase=str(b.DATA / 'catalog.sqlite'), controlDirectory=str(control))
    b.event('readyForNativeGui', source=str(b.WORK / 'fixture/source'), **b.check())
    while True:
        b.check()
        request = control / 'step.json'
        if not request.is_file():
            time.sleep(.1)
            continue
        value = json.loads(request.read_text())
        request.unlink()
        if value['op'] == 'snapshot':
            if value['label'] == 'imported' and not observe_foreground():
                continue
            focus.snapshot(value['label'])
            if value['label'] == 'imported':
                with closing(sqlite3.connect((b.DATA / 'catalog.sqlite').as_uri() + '?mode=ro', uri=True)) as db:
                    rows = db.execute('SELECT id,relative_path,filename FROM media_assets ORDER BY filename,id').fetchall()
                b.STATE['manifest'] = [dict(id=aid, relativePath=path) for aid, path, _ in rows]
                b.STATE['order'] = [dict(filename=name) for _, _, name in rows]
                b.save()
        elif value['op'] == 'foreground-preflight':
            observe_foreground()
        elif value['op'] == 'probe':
            assert value['count'] in (3, 100)
            result = probe_result(value['count'])
            b.STATE['latestProbe'] = result
            b.event('jointProbeObserved', count=value['count'], inputs=result['inputCount'],
                    screenTargets=len(result['screenDecodedTargetIds']),
                    domComplete=result['focusVariant']['completeDomObservation'],
                    osComplete=result['focusVariant']['osCorroboration']['complete'])
        elif value['op'] == 'window-pixels':
            try:
                observed = observe_window_pixels(value['index'])
            except (AssertionError, OSError, ValueError, subprocess.SubprocessError) as error:
                reason = str(error) or 'Isolated process or foreground identity changed'
                b.event('jointWindowPixelsUnverified', index=value['index'], reason=reason[:250])
            else:
                b.event('jointWindowPixelsObserved', **observed)
        elif value['op'] == 'finish':
            executable = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve())
            assert executable not in {line.strip() for line in subprocess.check_output(
                ['ps', '-axww', '-o', 'comm='], text=True).splitlines()}, 'Quit QA normally before finish'
            b.STATE['qaQuitNormally'] = True
            assert tuple(focus.completed) == focus.STEPS, f'Missing checkpoints: {focus.STEPS[len(focus.completed):]}'
            return
        else:
            raise ValueError('Unknown joint navigation request')


def observe_foreground():
    try:
        pid = confirm_foreground()
    except (AssertionError, OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        b.event('jointForegroundNotReady', reason=str(error)[:250] or 'Awaiting isolated empty checkpoint and probe')
        return False
    b.event('jointForegroundConfirmed', pid=pid)
    return True


def main():
    require_unlocked_console()
    assert not any(path.exists() or path.is_symlink() for path in [*reuse.OWNED, *b.EXTRA, b.GENERATED])
    subprocess.run([sys.executable, str(ROOT / 'scripts/check-test-artifact-budget.py'),
                    '--approval-file', str(b.APPROVAL), '--planned-source-files', '100',
                    '--planned-files', str(LIMITS['files']), '--planned-bytes', str(LIMITS['logicalBytes'])], check=True)
    assert shutil.disk_usage(ROOT).free >= LIMITS['logicalBytes']
    subprocess.run(['git', 'diff', '--exit-code', 'HEAD', '--', '.', ':!docs', ':!artifacts'], cwd=ROOT, check=True)
    b.STATE['sourceCommit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    b.STATE['normalBefore'] = reuse.normal_state()
    release = json.loads((ROOT / 'artifacts/build-info.json').read_text())
    executable = b.NORMAL / 'Contents/MacOS/travel-gallery'
    assert hashlib.sha256(executable.read_bytes()).hexdigest() == release['executableSha256']
    b.STATE['acceptedRelease'] = release['sha256']
    b.TEMP = tempfile.TemporaryDirectory(prefix='travel-gallery-native-navigation-focus-joint-')
    b.WORK = Path(b.TEMP.name).resolve()
    b.STATE['temporaryRoot'] = str(b.WORK)
    b.APPROVAL.write_text(json.dumps(dict(json.loads(b.APPROVAL.read_text()),
                                        temporaryRoot=str(b.WORK)), ensure_ascii=False, indent=2) + '\n')
    b.OWNS_GENERATED = True
    b.OWNED_PATHS.extend(reuse.OWNED)
    b.save()
    source = b.WORK / 'fixture/source'
    source.mkdir(parents=True)
    focus.write_sources(source)
    b.STATE['sourceBefore'] = reuse.source_state()
    (source.parent / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(dict(
        identifier=b.IDENTIFIER, count=100, fixtureRoot=str(source),
        scope='Owned numbered PNGs, navigation and focus observation only')))
    try:
        b.APP_PATH = b.build(True, b.prepare_home())
        run_gui()
        focus.focus_stop.set()
        focus.focus_thread.join(timeout=4)
        assert not focus.focus_thread.is_alive()
        result = probe_result(100)
        b.STATE['jointAnalysis'] = result
        b.save()
        focus.decode_screen_caches()
        assert result['focusVariant']['completeDomObservation'], 'Incomplete WebView focus/decode evidence'
        assert result['focusVariant']['osCorroboration']['complete'], 'Incomplete joint OS/WebView focus evidence'
        b.STATE['status'] = 'finished'
    finally:
        focus.focus_stop.set()
        if focus.focus_thread:
            focus.focus_thread.join(timeout=4)
        b.STATE['normalAfter'] = reuse.normal_state()
        b.STATE['normalPreserved'] = b.STATE['normalBefore'] == b.STATE['normalAfter']
        b.STATE['sourceUnchanged'] = reuse.source_state() == b.STATE['sourceBefore']
        b.STATE['releasePreserved'] = hashlib.sha256(executable.read_bytes()).hexdigest() == release['executableSha256']
        b.save()
        assert b.STATE['normalPreserved'] and b.STATE['sourceUnchanged'] and b.STATE['releasePreserved']


if __name__ == '__main__':
    argparse.ArgumentParser(description=__doc__).parse_args()
    assert not b.APPROVAL.exists() or json.loads(b.APPROVAL.read_text())['status'] == 'closed'
    b.APPROVAL.write_text(json.dumps(dict(
        status='active', limits=LIMITS, requiredFreeBytes=LIMITS['logicalBytes'],
        authorization='AGENTS.md 2026-11-23 standing resource approval; no fresh per-batch answer claimed',
        authorizationBasis='AGENTS.md:standing-resource-2026-11-23',
        authorizationFileSha256=hashlib.sha256((ROOT / 'AGENTS.md').read_bytes()).hexdigest(),
        cleanupPolicy='end-of-batch',
        scope='One offlineaudit performance observer build, 100 owned PNGs, 100 unique targets and 20 joint focus returns',
        cleanup='Stop owned processes and remove all run data on success, failure or interruption',
        recordedAt=datetime.now(timezone.utc).isoformat()), ensure_ascii=False, indent=2) + '\n')
    b.main = main
    timer = threading.Timer(2400, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        code = b.run()
        b.STATE['postCleanupBudget'] = json.loads(subprocess.check_output(
            [sys.executable, str(ROOT / 'scripts/check-test-artifact-budget.py')], text=True))
        b.save()
        raise SystemExit(code)
    finally:
        timer.cancel()
