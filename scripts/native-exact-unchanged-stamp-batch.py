"""Reproduce a same-size, same-mtime rewrite in the current exact duplicate flow."""
from contextlib import closing
import hashlib
import importlib.util
import json
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
b.APPROVAL = reuse.ROOT / 'docs/native-exact-unchanged-stamp-default-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-exact-unchanged-stamp-latest.json'
reuse.SCOPE = 'Native exact duplicates after a same-size same-mtime rewrite of one owned PNG'
reuse.RUN_PREFIX = 'travel-gallery-native-exact-unchanged-stamp-'
reuse.PLANNED_BYTES = 512 * 1024**2
reuse.SANDBOX_SEAL_ARGS = ('--webkit-mach-lookup',)
pngs = reuse.write_sources


def write_sources(source):
    pngs(source)
    data = (source / 'owned-0.png').read_bytes()
    for index in (1, 2):
        (source / f'owned-{index}.png').write_bytes(data)


def exact_groups(conn):
    rows = conn.execute("""
        SELECT g.id,a.relative_path FROM similar_groups g
        JOIN similar_group_items m ON m.group_id=g.id JOIN media_assets a ON a.id=m.asset_id
        WHERE g.group_type='exact' ORDER BY g.id,a.relative_path
    """)
    groups = {}
    for gid, name in rows:
        groups.setdefault(gid, []).append(name)
    return sorted(groups.values())


def restore_bytes(source):
    path = source / 'owned-0.png'
    data = (source / 'owned-1.png').read_bytes()
    before = next(row for row in b.STATE['sourceBefore'] if row['name'] == path.name)
    path.write_bytes(data)
    os.utime(path, ns=(path.stat().st_atime_ns, before['mtimeNs']))
    assert reuse.source_state() == b.STATE['sourceBefore']


def snapshot(label):
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    source = (b.WORK / 'fixture/source').resolve(strict=True)
    assert reuse.normal_state() == b.STATE['normalBefore']
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        assert conn.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not conn.execute('PRAGMA foreign_key_check').fetchall()
        assert not conn.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
        roots = conn.execute('SELECT id,absolute_path,is_online FROM source_roots').fetchall()
        assets = conn.execute('SELECT id,relative_path,sha256,byte_size,mtime_ms FROM media_assets ORDER BY id').fetchall()
        reviews = conn.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        scans = conn.execute("SELECT status,progress_done FROM jobs WHERE type='SCAN'").fetchall()
        hashes = conn.execute("""
            SELECT status,COALESCE(json_extract(checkpoint_json,'$.operation'),'manual')
            FROM jobs WHERE type='HASH_EXACT' ORDER BY created_at,rowid
        """).fetchall()
        groups = exact_groups(conn)
    if label == 'before-import':
        assert not roots and not assets and not reviews
        b.STATE['verifiedPid'] = pid
        b.event('isolatedEmptyCatalogVerified')
        return
    assert pid == b.STATE['verifiedPid'] and len(roots) == 1 and roots[0][1:] == (str(source), 1)
    assert len(assets) == len(reviews) == reuse.SOURCE_FILES
    identity = hashlib.sha256(json.dumps([roots, [row[:2] for row in assets], reviews]).encode()).hexdigest()
    initial = [['owned-0.png', 'owned-1.png', 'owned-2.png']]
    indexed = {name: sha for _, name, sha, _, _ in assets}
    if label == 'rewrite':
        assert groups == initial and hashes == [('completed', 'manual')]
        assert reuse.source_state() == b.STATE['sourceBefore']
        path = source / 'owned-0.png'
        replacement = (source / 'owned-3.png').read_bytes()
        old = path.stat()
        assert len(replacement) == old.st_size and replacement != path.read_bytes()
        b.STATE['catalogIdentitySha256'] = identity
        b.STATE['initialScans'] = len(scans)
        b.STATE['initialIndexedHash'] = indexed[path.name]
        path.write_bytes(replacement)
        os.utime(path, ns=(old.st_atime_ns, old.st_mtime_ns))
        new = path.stat()
        assert (new.st_size, new.st_mtime_ns, new.st_ino) == (old.st_size, old.st_mtime_ns, old.st_ino)
        assert new.st_ctime_ns != old.st_ctime_ns
        b.STATE['changedSource'] = reuse.source_state()
        b.event('sameStampRewrite', sameSize=True, sameNanosecondMtime=True, sameInode=True, ctimeChanged=True)
    elif label == 'after-query':
        assert identity == b.STATE['catalogIdentitySha256']
        assert reuse.source_state() == b.STATE['changedSource']
        assert len(scans) > b.STATE['initialScans'] and scans.count(('completed', 5)) == len(scans)
        assert groups == initial and hashes == [('completed', 'manual')]
        assert indexed['owned-0.png'] == b.STATE['initialIndexedHash'] != indexed['owned-3.png']
        assert hashlib.sha256((source / 'owned-0.png').read_bytes()).hexdigest() == indexed['owned-3.png']
        b.event('staleResultReproduced', explicitRescanCompleted=True, retainedMembers=groups[0],
                oldIndexedHashRetained=True, automaticRecomputeJobs=0)
    elif label == 'manual-control':
        assert identity == b.STATE['catalogIdentitySha256']
        assert reuse.source_state() == b.STATE['changedSource']
        assert hashes == [('completed', 'manual'), ('completed', 'manual')]
        assert groups == [['owned-0.png', 'owned-3.png'], ['owned-1.png', 'owned-2.png']]
        assert all(hashlib.sha256((source / name).read_bytes()).hexdigest() == sha for name, sha in indexed.items())
        restore_bytes(source)
        b.event('manualRehashControl', groups=groups, identitiesAndReviewsPreserved=True, ownedSourceRestored=True)
    else:
        raise ValueError(f'Unknown checkpoint: {label}')


reuse.write_sources, reuse.snapshot = write_sources, snapshot

if __name__ == '__main__':
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
