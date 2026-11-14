"""Verify how an isolated native story export handles a same-stamp rewrite."""
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
spec = importlib.util.spec_from_file_location(
    'preview_export', Path(__file__).with_name('native-export-after-preview-batch.py'))
export = importlib.util.module_from_spec(spec)
spec.loader.exec_module(export)
b, reuse, picker = export.b, export.reuse, export.picker
pending = None


def restore():
    global pending
    if pending is not None:
        path, data, before = pending
        path.write_bytes(data)
        os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
        pending = None
        assert reuse.source_state() == b.STATE['sourceBefore']


def arm(conn, source, output, require_versions=False):
    global pending
    assert pending is None and not any(output.iterdir())
    assert reuse.source_state() == b.STATE['sourceBefore']
    plans = conn.execute("""
        SELECT plan_json,count_files,consumed_by_job_id FROM export_plans
        WHERE json_extract(plan_json,'$.operation')='storyExport'
    """).fetchall()
    assert len(plans) == 1 and plans[0][1:] == (5, None)
    plan = json.loads(plans[0][0])
    assert Path(plan['destination']) == output and not plan['includeNotes']
    assert {item['filename'] for item in plan['items']} == export.MEDIA
    if require_versions:
        indexed = dict(conn.execute('SELECT id,file_version FROM media_assets'))
        assert all(item['sourceVersion'] and item['sourceVersion'] == indexed[item['assetId']]
                   for item in plan['items'])
    cache = dict(conn.execute("SELECT asset_id,cache_rel_path FROM thumbnail_cache WHERE variant='screen'"))
    item = next(item for item in plan['items'] if item['assetId'] in cache)
    path = source / item['filename']
    donor = next(other for other in sorted(source.glob('*.png')) if other != path)
    original, replacement = path.read_bytes(), donor.read_bytes()
    before = path.stat()
    assert before.st_size == len(original) == len(replacement) == 70 and original != replacement
    assert item['mtime'] == before.st_mtime_ns // 1_000_000
    pending = path, original, before
    path.write_bytes(replacement)
    os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
    after = path.stat()
    assert (after.st_size, after.st_ino, after.st_mtime_ns) == (before.st_size, before.st_ino, before.st_mtime_ns)
    assert after.st_ctime_ns != before.st_ctime_ns
    b.STATE['storyCheck'] = dict(folder=plan['folder'], relative=item['relative'],
                                 asset=item['assetId'], filename=path.name, donor=donor.name)
    b.event('storySourceRewrittenAfterPreview', name=path.name, warmScreenCache=True,
            sameSizeMtimeAndInode=True, changeTimeAdvanced=True)


def verify(conn, output, expected_failures=None):
    check = b.STATE['storyCheck']
    assert pending is not None
    rows = conn.execute("""
        SELECT j.id,j.status,j.progress_done,j.failure_count FROM jobs j
        WHERE json_extract(j.checkpoint_json,'$.operation')='storyExport'
    """).fetchall()
    assert len(rows) == 1 and rows[0][1:3] == ('completed', 7)
    job, _, _, failures = rows[0]
    assert failures in (0, 1)
    if expected_failures is not None:
        assert failures == expected_failures
    folder = output / check['folder']
    manifest = json.loads((folder / 'manifest.json').read_text())
    assert manifest['itemCount'] == 5 and manifest['previewCount'] == 5 - failures
    assert manifest['failedCount'] == failures and not manifest['includesNotes']
    assert not manifest['originalsIncluded']
    exported = verify_files(folder, manifest, 6 - failures)
    rows = conn.execute("""
        SELECT asset_id,dest_relative_path,state,error_code,verified_sha256
        FROM export_run_items WHERE job_id=?
    """, (job,)).fetchall()
    assert len(rows) == 5
    html = (folder / 'index.html').read_text()
    for asset, relative, state, code, sha in rows:
        name = Path(relative).relative_to(check['folder']).as_posix()
        if asset == check['asset'] and failures:
            assert (state, code, sha) == ('failed', 'DB_CONFLICT', None)
            assert name not in exported
        else:
            assert state == 'copied' and code is None and sha == exported[name]
            assert f'src="{name}"' in html
    assert (f'src="{check["relative"]}"' in html) == (not failures)
    assert ('此项预览未导出' in html) == bool(failures)
    restore()
    b.event('storyVersionOutcome', changedItemPublished=not failures, previews=5-failures,
            failedItems=failures, manifestFiles=len(exported), htmlAndHashesVerified=True,
            sourceRestored=True)


def verify_files(folder, manifest, expected_count):
    files = {item['relative']: item for item in manifest['files']}
    assert len(files) == len(manifest['files']) == expected_count and 'index.html' in files
    assert {path.name for path in folder.iterdir()} == {*files, 'manifest.json'}
    for name, item in files.items():
        content = (folder / name).read_bytes()
        assert len(content) == item['bytes'] and hashlib.sha256(content).hexdigest() == item['sha256']
        if name != 'index.html':
            assert name.endswith('.jpg') and content.startswith(b'\xff\xd8') and content.endswith(b'\xff\xd9')
    return {name: item['sha256'] for name, item in files.items()}


def verify_recovery(conn, output):
    if 'initialScanIds' in b.STATE:
        scans = successful_scans(conn)
        assert scans > set(b.STATE['initialScanIds'])
    rows = conn.execute("""
        SELECT id,checkpoint_json,status,progress_done,failure_count FROM jobs
        WHERE json_extract(checkpoint_json,'$.operation')='storyExport'
        ORDER BY created_at,rowid
    """).fetchall()
    assert len(rows) == 2 and rows[0][2:] == ('completed', 7, 1)
    job, data, status, done, failed_count = rows[1]
    assert (status, done, failed_count) == ('completed', 7, 0)
    plan = json.loads(data)
    folder = output / plan['folder']
    assert folder != output / b.STATE['storyCheck']['folder']
    manifest = json.loads((folder / 'manifest.json').read_text())
    assert (manifest['itemCount'], manifest['previewCount'], manifest['failedCount']) == (5, 5, 0)
    assert not manifest['includesNotes'] and not manifest['originalsIncluded']
    files = verify_files(folder, manifest, 6)
    html = (folder / 'index.html').read_text()
    assert '此项预览未导出' not in html
    items = conn.execute("""
        SELECT dest_relative_path,state,error_code,verified_sha256 FROM export_run_items WHERE job_id=?
    """, (job,)).fetchall()
    assert len(items) == 5
    for relative, state, error, sha in items:
        name = Path(relative).relative_to(plan['folder']).as_posix()
        assert (state, error, sha) == ('copied', None, files[name])
        assert f'src="{name}"' in html
    assert reuse.source_state() == b.STATE['sourceBefore']
    b.event('storyRescanRecoveryVerified', previews=5, failedItems=0, manifestFiles=6,
            scanVerified='initialScanIds' in b.STATE)


def successful_scans(conn):
    return {row[0] for row in conn.execute("""SELECT id FROM jobs
        WHERE type='SCAN' AND status='completed' AND progress_done=5 AND failure_count=0""")}


def snapshot(label):
    if label in ('before-import', 'after-bootstrap'):
        export.snapshot(label)
        return
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    assert reuse.normal_state() == b.STATE['normalBefore']
    assert label == 'after-import' or pid == b.STATE['verifiedPid']
    fixture = (b.WORK / 'fixture').resolve(strict=True)
    with closing(sqlite3.connect(database.as_uri()+'?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        assert conn.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not conn.execute('PRAGMA foreign_key_check').fetchall()
        if label == 'after-import':
            assert conn.execute('SELECT absolute_path FROM source_roots').fetchall() == [(str(fixture/'source'),)]
            assert {row[0] for row in conn.execute("SELECT relative_path FROM media_assets WHERE file_state='available'")} == export.MEDIA
            scans = successful_scans(conn)
            assert scans
            b.STATE.update(verifiedPid=pid, initialScanIds=sorted(scans))
            b.event('nativePickerSourceScanned', sourceCount=len(export.MEDIA))
        elif label == 'arm':
            arm(conn, fixture/'source', fixture/'output', require_versions='initialScanIds' in b.STATE)
        elif label == 'verify':
            verify(conn, fixture/'output')
        elif label == 'verify-fixed':
            verify(conn, fixture/'output', expected_failures=1)
        elif label == 'recovery':
            verify_recovery(conn, fixture/'output')
        else:
            raise ValueError(f'Unknown story checkpoint: {label}')


if __name__ == '__main__':
    b.APPROVAL = reuse.ROOT / 'docs/native-story-version-default-budget.json'
    b.RECEIPT = reuse.ROOT / 'docs/native-story-version-latest.json'
    reuse.SCOPE = 'Native story export after a same-stamp rewrite of a source with a screen preview'
    reuse.RUN_PREFIX = 'travel-gallery-native-story-version-'
    reuse.SANDBOX_SEAL_ARGS = ('--webkit-mach-lookup',)
    export.same_stamp = True
    reuse.write_sources, reuse.snapshot = export.write_sources, snapshot
    cleanup = reuse.RunDirectory.cleanup

    def cleanup_with_restore(run):
        try:
            restore()
        finally:
            cleanup(run)

    reuse.RunDirectory.cleanup = cleanup_with_restore
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
