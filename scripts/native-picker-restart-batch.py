"""Native restart with real picker authorization and no static source-path entitlement."""
from contextlib import closing
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
from threading import Timer

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
b = reuse.b
b.APPROVAL = reuse.ROOT / 'docs/native-picker-restart-default-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-picker-restart-latest.json'
reuse.SCOPE = 'macOS native directory picker, same catalog across restart, no fixed source-path entitlement'
reuse.RUN_PREFIX = 'travel-gallery-native-picker-restart-'
reuse.PLANNED_BYTES = 512 * 1024**2
reuse.SANDBOX_SEAL_ARGS += ('--picker-only',)


def verified_process(database):
    executable = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve(strict=True))
    processes = subprocess.check_output(['ps', '-axww', '-o', 'pid=,comm='], text=True, errors='replace')
    pids = [int(pid) for line in processes.splitlines()
            for pid, command in [line.strip().split(None, 1)] if command == executable]
    if len(pids) != 1:
        raise RuntimeError('Expected exactly one live QA process at the isolated executable path')
    lock = database.with_name('catalog.lock')
    opened = subprocess.check_output(['/usr/sbin/lsof', '-a', '-p', str(pids[0]), '-Fn'], text=True)
    if not database.is_file() or database.is_symlink() or lock.is_symlink() or 'n' + str(lock.resolve()) not in opened.splitlines():
        raise RuntimeError('QA process is not using the isolated catalog')
    return pids[0]


def verify_reauthorization(label, online, states, scans, baseline):
    """Check the catalog transitions independently of the visible QA steps."""
    successes = [scan_id for scan_id, status, count, failures, _ in scans
                 if (status, count, failures) == ('completed', reuse.SOURCE_FILES, 0)]
    if label == 'before-restart':
        assert online and states == {'available'} and successes and len(successes) == len(scans)
        baseline['initialScans'] = successes
    elif label in ('after-restart', 'after-denied-rescan'):
        assert not online and states == {'offline'}
        new = [scan for scan in scans if scan[0] not in baseline['initialScans']]
        if label == 'after-restart':
            assert not new, 'No automatic scan should run with a permission-denied root'
        else:
            assert len(new) == 1
            scan_id, status, count, _, reason = new[0]
            assert (status, count) == ('failed', 0) and '重新授权' in reason
            baseline['deniedScan'] = scan_id
    elif label == 'after-reselect':
        assert online and states == {'available'}
        assert set(successes) > set(baseline['initialScans'])
        assert {scan[0] for scan in scans} - set(successes) == {baseline['deniedScan']}


def snapshot(label):
    preparation = json.loads((b.WORK / 'qa/preparation.json').read_text())
    entitlements = preparation['parentEntitlements']
    assert preparation['pickerOnly'] and entitlements['com.apple.security.files.user-selected.read-write']
    assert not any('absolute-path' in name for name in entitlements)
    database = b.DATA / 'catalog.sqlite'
    pid = verified_process(database)
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        roots = conn.execute('SELECT id,absolute_path,is_online FROM source_roots ORDER BY id').fetchall()
        assets = conn.execute('SELECT id,relative_path,file_state FROM media_assets ORDER BY id').fetchall()
        reviews = conn.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        reviewed = conn.execute("SELECT count(*) FROM asset_reviews WHERE rating=4 AND pick_state='keep'").fetchone()[0]
        cached = conn.execute('SELECT count(*) FROM thumbnail_cache').fetchone()[0]
        scans = conn.execute("SELECT id,status,progress_done,failure_count,COALESCE(error_summary,'') FROM jobs WHERE type='SCAN'").fetchall()
        assert conn.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not conn.execute('PRAGMA foreign_key_check').fetchall()
        assert not conn.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused') LIMIT 1").fetchone()
    if label == 'before-import':
        assert not roots and not assets and not reviews
    else:
        source = str((b.WORK / 'fixture/source').resolve(strict=True))
        assert len(roots) == 1 and roots[0][1] == source and len(assets) == reuse.SOURCE_FILES
        assert reviewed == 1 and len(reviews) == len(assets)
        unchanged = hashlib.sha256(json.dumps([roots[0][:2], [a[:2] for a in assets], reviews]).encode()).hexdigest()
        if label == 'before-restart':
            assert roots[0][2] and all(a[2] == 'available' for a in assets) and cached >= len(assets)
            b.STATE['originalPid'] = pid
            b.STATE['catalogIdentitySha256'] = unchanged
        else:
            assert label in ('after-restart', 'after-denied-rescan', 'after-reselect') and pid != b.STATE['originalPid']
            assert unchanged == b.STATE['catalogIdentitySha256']
            if label == 'after-reselect':
                assert roots[0][2] and all(a[2] == 'available' for a in assets)
    assert reuse.source_state() == b.STATE['sourceBefore']
    assert reuse.normal_state() == b.STATE['normalBefore']
    online = bool(roots and roots[0][2])
    states = {asset[2] for asset in assets}
    if b.STATE.get('expectReauthorization'):
        verify_reauthorization(label, online, states, scans, b.STATE.setdefault('scanCheckpoints', {}))
    b.event('pickerRestartSnapshot', label=label, actualPid=pid, sources=len(roots),
            assets=len(assets), states=sorted(states),
            online=online, marked=reviewed, cacheReferences=cached,
            scans=[dict(status=status, progress=count, failures=failures) for _, status, count, failures, _ in scans],
            pickerOnly=True, catalogIdentityPreserved=label != 'before-import')


reuse.snapshot = snapshot

if __name__ == '__main__':
    b.STATE['controllerPid'] = os.getpid()
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
