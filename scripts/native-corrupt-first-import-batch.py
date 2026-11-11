"""Verify the first native scan of an owned mix of good, corrupt and empty images."""
from contextlib import closing
import importlib.util
import os
from pathlib import Path
import signal
import sqlite3
import sys
from threading import Timer

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('picker', Path(__file__).with_name('native-picker-restart-batch.py'))
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-corrupt-first-import-default-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-corrupt-first-import-latest.json'
reuse.SCOPE = 'First native import of three decodable images, a corrupt image and a zero-byte file'
reuse.RUN_PREFIX = 'travel-gallery-native-corrupt-first-import-'
reuse.PLANNED_BYTES = 512 * 1024**2
write_pngs = reuse.write_sources
GOOD = {'owned-0.png', 'owned-1.png', 'owned-2.png'}
BAD = {'broken.png', 'zero-byte.jpg'}


def write_sources(source):
    write_pngs(source)
    (source / 'owned-3.png').rename(source / 'broken.png')
    (source / 'broken.png').write_bytes(b'\x89PNG\r\n\x1a\ntruncated')
    (source / 'owned-4.png').rename(source / 'zero-byte.jpg')
    (source / 'zero-byte.jpg').write_bytes(b'')


def snapshot(label):
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    source = (b.WORK / 'fixture/source').resolve(strict=True)
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        roots = conn.execute('SELECT absolute_path,is_online FROM source_roots').fetchall()
        assets = conn.execute('SELECT relative_path,file_state,byte_size FROM media_assets ORDER BY relative_path').fetchall()
        jobs = conn.execute(
            'SELECT id,type,status,progress_done,progress_total,failure_count FROM jobs ORDER BY created_at,rowid'
        ).fetchall()
        errors = conn.execute(
            'SELECT e.job_id,a.relative_path,e.message FROM job_errors e JOIN media_assets a ON a.id=e.asset_id ORDER BY a.relative_path'
        ).fetchall()
        cached = conn.execute(
            "SELECT a.relative_path FROM thumbnail_cache t JOIN media_assets a ON a.id=t.asset_id WHERE t.variant='small'"
        ).fetchall()
        assert conn.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not conn.execute('PRAGMA foreign_key_check').fetchall()
    if label == 'before-import':
        assert not roots and not assets and not jobs and not errors and not cached, 'Initial QA catalog must be empty'
        b.STATE['verifiedPid'] = pid
    elif label == 'after-import':
        assert pid == b.STATE['verifiedPid'], 'Import must complete in the verified process'
        assert roots == [(str(source), 1)]
        assert {name: state for name, state, _ in assets} == {
            **dict.fromkeys(GOOD, 'available'), **dict.fromkeys(BAD, 'corrupt')
        }, assets
        assert {name for name, _, size in assets if size == 0} == {'zero-byte.jpg'}
        assert not any(row[2] in ('queued', 'running', 'paused') for row in jobs)
        scans = [job for job in jobs if job[1] == 'SCAN']
        assert scans and scans[0][2:] == ('completed', 5, 5, 2), jobs
        scan_errors = [(name, message) for jid, name, message in errors if jid == scans[0][0]]
        assert len(scan_errors) == 2 and {name for name, _ in scan_errors} == BAD, errors
        assert all('媒体损坏' in message for _, message in scan_errors)
        assert {name for (name,) in cached} == GOOD, cached
    else:
        raise ValueError(f'Unknown checkpoint: {label}')
    assert reuse.source_state() == b.STATE['sourceBefore']
    assert reuse.normal_state() == b.STATE['normalBefore']
    b.event('firstImportSnapshot', label=label, actualPid=pid, sources=len(roots), assets=assets,
            tasks=[job[1:] for job in jobs], itemErrors=[row[1:] for row in errors],
            cachedPreviews=sorted(name for (name,) in cached), sourceAndFormalUnchanged=True)


reuse.write_sources, reuse.snapshot = write_sources, snapshot

if __name__ == '__main__':
    b.STATE['controllerPid'] = os.getpid()
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
