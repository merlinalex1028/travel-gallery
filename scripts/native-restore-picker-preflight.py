"""Check real directory/file picker authorization with the accepted isolated App."""
from contextlib import closing
import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


picker = load('native-picker-restart-batch')
reuse, b = picker.reuse, picker.b
guard = load('native-gui-preflight')
b.APPROVAL = ROOT / 'docs/native-restore-picker-preflight-budget.json'
b.RECEIPT = ROOT / 'docs/native-restore-picker-preflight-latest.json'
reuse.SCOPE = 'Accepted isolated App: real import/backup directory and restore file pickers; no build or restore'
reuse.RUN_PREFIX = 'travel-gallery-restore-picker-preflight-'
reuse.PLANNED_BYTES = 512 * 1024**2
reuse.SANDBOX_SEAL_ARGS = ('--webkit-mach-lookup', '--picker-only')
original_sources = reuse.write_sources


def write_sources(source):
    original_sources(source)
    (source.parent.parent / 'backup').mkdir()


def accept():
    control = b.WORK / 'control'
    b.STATE['controlDirectory'] = str(control)
    database = b.DATA / 'catalog.sqlite'
    pid = None
    for step in ('empty', 'imported', 'backed-up', 'file-reviewing', 'finish'):
        b.STATE['expectedCheckpoint'] = step
        b.event('awaitingNativeCheckpoint', step=step)
        request = control / 'step.json'
        deadline = time.monotonic() + 900
        while not request.is_file():
            guard.require_unlocked_console()
            b.check()
            if time.monotonic() >= deadline:
                raise TimeoutError(f'No native picker checkpoint: {step}')
            time.sleep(.25)
        observed = json.loads(request.read_text())
        request.unlink()
        assert observed['step'] == step
        assert reuse.normal_state() == b.STATE['normalBefore']
        assert reuse.source_state() == b.STATE['sourceBefore']
        if step == 'finish':
            executable = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve())
            assert executable not in subprocess.check_output(
                ['ps', '-axww', '-o', 'comm='], text=True).splitlines()
            b.STATE.update(qaQuitNormally=True, pickerPreflightPassed=True)
            b.STATE.pop('expectedCheckpoint')
            return
        current_pid = picker.verified_process(database)
        assert pid is None or current_pid == pid
        pid = current_pid
        with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as db:
            db.execute('BEGIN')
            assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
            count = db.execute('SELECT count(*) FROM media_assets').fetchone()[0]
            roots = db.execute('SELECT absolute_path FROM source_roots').fetchall()
            assert not db.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
        if step == 'empty':
            assert count == 0 and not roots
            asn = subprocess.check_output(
                ['lsappinfo', '-nonames', 'find', f'pid={pid}'], text=True, timeout=3).strip()
            front = subprocess.check_output(
                ['lsappinfo', '-nonames', 'front'], text=True, timeout=3).strip()
            b.event('nativeForegroundObserved', pid=pid, applicationAsn=asn, frontAsn=front)
            assert asn.startswith('ASN:') and front == asn, 'Verify actual QA foreground before import'
        else:
            assert count == 5 and roots == [(str(b.WORK / 'fixture/source'),)]
        if step == 'backed-up':
            [backup] = list((b.WORK / 'backup').glob('travel-gallery-*.sqlite'))
            portable = b.WORK / 'picker-portable.sqlite'
            with closing(sqlite3.connect(backup.as_uri() + '?immutable=1', uri=True)) as src:
                with closing(sqlite3.connect(portable)) as dst:
                    src.backup(dst)
                    assert dst.execute('PRAGMA journal_mode=DELETE').fetchone() == ('delete',)
                    assert dst.execute('SELECT count(*) FROM media_assets').fetchone() == (5,)
            b.STATE.update(originalBackup=str(backup), pickerPortableBackup=str(portable))
        if step == 'file-reviewing':
            frame = observed['axFrame']
            assert b.STATE['pickerPortableBackup'] in frame and '备份当前库并恢复' in frame
            assert not list(b.DATA.glob('before-restore-*.sqlite'))
        b.event('nativePickerPreflightCheckpoint', step=step, pid=pid, assets=count,
                axFrame=observed.get('axFrame'))


if __name__ == '__main__':
    guard.require_unlocked_console()
    reuse.write_sources = write_sources
    original_main = reuse.main
    reuse.main = lambda: original_main(on_ready=accept)
    raise SystemExit(reuse.run())
