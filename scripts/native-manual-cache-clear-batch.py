"""Check that an out-of-app thumbnail cache deletion leaves sources and reviews intact."""
from contextlib import closing
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import sys
from threading import Timer

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('picker', Path(__file__).with_name('native-picker-restart-batch.py'))
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-manual-cache-clear-default-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-manual-cache-clear-latest.json'
reuse.SCOPE = 'Manual deletion of the stopped QA thumbnail cache, then restart and rebuild'
reuse.RUN_PREFIX = 'travel-gallery-native-manual-cache-clear-'
reuse.PLANNED_BYTES = 512 * 1024**2
reuse.SANDBOX_SEAL_ARGS = ('--webkit-mach-lookup',)


def cached_files(cache, rows):
    assert cache.is_dir() and not cache.is_symlink()
    files = {}
    for aid, variant, relative, created in rows:
        path = cache / relative
        assert not path.is_symlink() and path.resolve(strict=True).is_relative_to(cache.resolve(strict=True))
        contents = path.read_bytes()
        assert contents.startswith(b'\xff\xd8') and contents.endswith(b'\xff\xd9'), (aid, variant)
        files[(aid, variant)] = (hashlib.sha256(contents).hexdigest(), created)
    return files


def snapshot(label):
    database = b.DATA / 'catalog.sqlite'
    cache = b.CACHE / 'thumbs'
    live = label != 'clear-while-closed'
    if live:
        pid = picker.verified_process(database)
    else:
        app = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve(strict=True))
        processes = subprocess.check_output(['ps', '-axww', '-o', 'comm='], text=True).splitlines()
        assert app not in {path.strip() for path in processes}, 'Quit the isolated app before manually clearing'
    assert reuse.normal_state() == b.STATE['normalBefore']
    assert reuse.source_state() == b.STATE['sourceBefore']
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        roots = conn.execute('SELECT id,absolute_path,is_online FROM source_roots').fetchall()
        assets = conn.execute('SELECT id,relative_path,file_state FROM media_assets ORDER BY id').fetchall()
        reviews = conn.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        marks = conn.execute("SELECT count(*) FROM asset_reviews WHERE rating=4 AND pick_state='keep'").fetchone()[0]
        rows = conn.execute('SELECT asset_id,variant,cache_rel_path,created_at FROM thumbnail_cache').fetchall()
        assert conn.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not conn.execute('PRAGMA foreign_key_check').fetchall()
        assert not conn.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
    if label == 'before-import':
        assert not roots and not assets and not reviews and not rows
        b.STATE['originalPid'] = pid
        b.event('emptyIsolatedCatalogVerified')
        return
    assert len(roots) == 1 and roots[0][1:] == (str((b.WORK / 'fixture/source').resolve()), 1)
    assert len(assets) == len(reviews) == reuse.SOURCE_FILES and marks == 1
    assert all(state == 'available' for _, _, state in assets)
    catalog = hashlib.sha256(json.dumps([roots, assets, reviews]).encode()).hexdigest()
    files = cached_files(cache, rows)
    if label == 'before-clear':
        assert pid == b.STATE['originalPid']
        assert {aid for aid, variant in files if variant == 'small'} == {aid for aid, _, _ in assets}
        assert any(variant == 'screen' for _, variant in files)
        b.STATE['catalogBeforeClear'] = catalog
        b.STATE['cachedBeforeClear'] = [[aid, variant, sha, created] for (aid, variant), (sha, created) in files.items()]
        b.event('catalogBeforeManualClear', media=len(assets), marked=marks,
                small=sum(variant == 'small' for _, variant in files),
                screen=sum(variant == 'screen' for _, variant in files))
        return
    assert catalog == b.STATE['catalogBeforeClear']
    baseline = {(aid, variant): (sha, created) for aid, variant, sha, created in b.STATE['cachedBeforeClear']}
    if label == 'clear-while-closed':
        assert files == baseline and not b.STATE.get('manualClearDone')
        expected_parent = b.LIBRARY / 'Caches' / 'com.travelgallery.desktop'
        assert cache.parent == expected_parent and not any(path.is_symlink() for path in (cache, *cache.parents))
        database_sha = hashlib.sha256(database.read_bytes()).hexdigest()
        shutil.rmtree(cache)
        assert not cache.exists() and hashlib.sha256(database.read_bytes()).hexdigest() == database_sha
        assert reuse.source_state() == b.STATE['sourceBefore']
        b.STATE['manualClearDone'] = True
        b.event('manualCacheDeleted', priorReferences=len(rows), databaseBytesPreserved=True, sourceUnchanged=True)
    elif label == 'after-restart':
        assert b.STATE['manualClearDone'] and pid != b.STATE['originalPid']
        assert files.keys() >= baseline.keys()
        for key, (sha, created) in baseline.items():
            assert files[key][0] == sha and files[key][1] > created
        b.event('cacheRebuilt', sameCatalogAndReviews=True, sameSourceFiles=True,
                rebuiltSmall=sum(variant == 'small' for _, variant in baseline),
                rebuiltScreen=sum(variant == 'screen' for _, variant in baseline),
                rebuiltCacheBytesMatched=True)
    else:
        raise ValueError(f'Unknown checkpoint: {label}')


reuse.snapshot = snapshot

if __name__ == '__main__':
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
