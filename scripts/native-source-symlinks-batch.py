"""Observe native source scans across an owned symlink cycle and source boundary."""
from contextlib import closing
import hashlib
import importlib.util
import os
from pathlib import Path
import signal
import sqlite3
import sys
from threading import Timer

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
b = reuse.b
b.APPROVAL = reuse.ROOT / 'docs/native-source-symlinks-default-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-source-symlinks-latest.json'
reuse.SCOPE = 'Current sealed macOS QA copy, source scan with a symlink loop and out-of-root links'
reuse.RUN_PREFIX = 'travel-gallery-native-source-symlinks-'
reuse.PLANNED_BYTES = 512 * 1024**2
regular_sources = ['nested/owned-1.png', 'owned-0.png', 'owned-2.png']
write_pngs = reuse.write_sources


def write_sources(source):
    write_pngs(source)
    held = source.with_name('held-out')
    held.mkdir()
    (source / 'nested').mkdir()
    (source / 'owned-1.png').rename(source / 'nested/owned-1.png')
    for name in ('owned-3.png', 'owned-4.png'):
        (source / name).rename(held / name)
    (source / 'nested/cycle').symlink_to('..', target_is_directory=True)
    (source / 'linked-folder').symlink_to('../held-out', target_is_directory=True)
    (source / 'linked-external.png').symlink_to('../held-out/owned-3.png')
    (source / 'linked-internal.png').symlink_to('owned-0.png')


def source_state():
    fixture = b.WORK / 'fixture'
    rows = []
    for base, dirs, files in os.walk(fixture, followlinks=False):
        for name in [*dirs, *files]:
            path = Path(base) / name
            relative = path.relative_to(fixture).as_posix()
            if relative.split('/')[0] not in ('source', 'held-out'):
                continue
            if path.is_symlink():
                rows.append(dict(name=relative, link=os.readlink(path)))
            elif path.is_file():
                stat = path.stat()
                rows.append(dict(name=relative, sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                                 bytes=stat.st_size, mtimeNs=stat.st_mtime_ns, inode=stat.st_ino))
    return sorted(rows, key=lambda row: row['name'])


def snapshot(label):
    database = b.DATA / 'catalog.sqlite'
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        assets = conn.execute('SELECT id,relative_path,file_state FROM media_assets ORDER BY relative_path').fetchall()
        jobs = conn.execute('SELECT type,status,progress_done,failure_count FROM jobs ORDER BY created_at,id').fetchall()
        count = conn.execute('SELECT count(*) FROM source_roots').fetchone()[0]
    if label == 'before-import':
        assert count == 0 and not assets and not jobs
    elif label in ('initial-scan', 'explicit-rescan'):
        assert count == 1
        assert [(path, state) for _, path, state in assets] == [(path, 'available') for path in regular_sources]
        scans = [job for job in jobs if job[0] == 'SCAN']
        assert len(scans) >= (2 if label == 'explicit-rescan' else 1)
        assert all(job[1:] == ('completed', 3, 0) for job in scans)
        assert not any(job[1] in ('queued', 'running', 'paused') for job in jobs)
        ids = [asset[0] for asset in assets]
        if label == 'initial-scan':
            b.STATE['initialAssetIds'] = ids
        else:
            assert ids == b.STATE['initialAssetIds']
    else:
        raise ValueError(f'Unknown checkpoint: {label}')
    assert source_state() == b.STATE['sourceBefore']
    assert reuse.normal_state() == b.STATE['normalBefore']
    b.event('sourceLinkBoundary', label=label, sources=count, assets=[row[1] for row in assets],
            scans=[row for row in jobs if row[0] == 'SCAN'], sourcesAndLinkTargetsPreserved=True)


reuse.write_sources, reuse.source_state, reuse.snapshot = write_sources, source_state, snapshot

if __name__ == '__main__':
    b.STATE['controllerPid'] = os.getpid()
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
