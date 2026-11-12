"""Exercise native export after changes to one source and two destinations."""
import argparse
from contextlib import closing
import csv
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import sqlite3
import sys
from threading import Timer
import xml.etree.ElementTree as ET

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('picker', Path(__file__).with_name('native-picker-restart-batch.py'))
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-export-after-preview-default-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-export-after-preview-latest.json'
reuse.SCOPE = 'Native export after source change and destination conflicts following preview'
reuse.RUN_PREFIX = 'travel-gallery-native-export-after-preview-'
reuse.PLANNED_BYTES = 512 * 1024**2
write_pngs = reuse.write_sources
MEDIA = {f'owned-{index}.png' for index in range(reuse.SOURCE_FILES)}
SENTINELS = {'owned-1.xmp': b'external XMP conflict', 'owned-2.png': b'external media conflict'}
changed = None
same_stamp = False


def write_sources(source):
    write_pngs(source)
    (source.parent / 'output').mkdir()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def restore_source():
    global changed
    if changed is not None:
        path, original, atime, mtime = changed
        path.write_bytes(original)
        os.utime(path, ns=(atime, mtime))
        changed = None


def arm_race(conn, source, output):
    global changed
    assert changed is None and not any(output.iterdir())
    assert reuse.source_state() == b.STATE['sourceBefore']
    plans = conn.execute('SELECT plan_json,count_files,consumed_by_job_id FROM export_plans').fetchall()
    assert len(plans) == 1 and plans[0][1:] == (len(MEDIA), None)
    plan = json.loads(plans[0][0])
    assert Path(plan['destination']) == output and plan['policy'] == 'rename'
    assert {item['relative'] for item in plan['items']} == MEDIA
    assert all(not item['skip'] and not item['unavailable'] and item['size'] == 70 for item in plan['items'])
    assert {item['xmp']['relative'] for item in plan['items']} == {Path(name).with_suffix('.xmp').name for name in MEDIA}
    path = source / 'owned-0.png'
    original = path.read_bytes()
    stat = path.stat()
    changed = path, original, stat.st_atime_ns, stat.st_mtime_ns
    if same_stamp:
        replacement = (source / 'owned-3.png').read_bytes()
        item = next(item for item in plan['items'] if item['relative'] == path.name)
        assert len(replacement) == len(original) and replacement != original
        assert item['mtime'] == stat.st_mtime_ns // 1_000_000
        path.write_bytes(replacement)
        os.utime(path, ns=(stat.st_atime_ns, stat.st_mtime_ns))
        modified = path.stat()
        assert (modified.st_size, modified.st_mtime_ns, modified.st_ino) == (
            stat.st_size, stat.st_mtime_ns, stat.st_ino)
        assert modified.st_ctime_ns != stat.st_ctime_ns
        b.event('sameStampRewriteAfterPreview', plannedMedia=len(MEDIA), plannedXmp=len(MEDIA),
                changedSource=path.name, sameSize=True, sameNanosecondMtime=True, sameInode=True,
                ctimeChanged=True, previewSourceSha256=hashlib.sha256(original).hexdigest(),
                changedSourceSha256=digest(path))
        return
    with path.open('ab') as stream:
        stream.write(b'changed after preview')
    for name, data in SENTINELS.items():
        with (output / name).open('xb') as stream:
            stream.write(data)
    b.event('raceArmedAfterPreview', plannedMedia=len(MEDIA), plannedXmp=len(MEDIA),
            changedSource=path.name, conflictingTargets=sorted(SENTINELS))


def verify_export(conn, source, output):
    assert changed is not None
    jobs = conn.execute("SELECT id,status,progress_done,progress_total,failure_count FROM jobs WHERE type='EXPORT'").fetchall()
    assert len(jobs) == 1 and jobs[0][1:4] == ('completed', 5, 5), jobs
    job = jobs[0][0]
    manifest = output / f'travel-gallery-{job}.csv'
    expected = {
        ('media', 'owned-0.png'): ('failed', 'DB_CONFLICT'),
        ('media', 'owned-2.png'): ('failed', 'EXPORT_CONFLICT'),
        ('xmp', 'owned-0.xmp'): ('skipped', 'MEDIA_EXPORT_FAILED'),
        ('xmp', 'owned-1.xmp'): ('failed', 'EXPORT_CONFLICT'),
        ('xmp', 'owned-2.xmp'): ('skipped', 'MEDIA_EXPORT_FAILED'),
    }
    copied = {('media', name) for name in ('owned-1.png', 'owned-3.png', 'owned-4.png')}
    copied |= {('xmp', name) for name in ('owned-3.xmp', 'owned-4.xmp')}
    if same_stamp:
        assert jobs[0][4] in (0, 1), jobs
        expected = {}
        copied = {('media', name) for name in MEDIA}
        copied |= {('xmp', Path(name).with_suffix('.xmp').name) for name in MEDIA}
        if jobs[0][4] == 1:
            expected = {('media', 'owned-0.png'): ('failed', 'DB_CONFLICT'),
                        ('xmp', 'owned-0.xmp'): ('skipped', 'MEDIA_EXPORT_FAILED')}
            copied.difference_update(expected)
    else:
        assert jobs[0][4] == 3, jobs
    expected.update(dict.fromkeys(copied, ('copied', '')))
    with manifest.open(encoding='utf-8-sig', newline='') as stream:
        reports = {(row['type'], row['filename']): row for row in csv.DictReader(stream)}
    actual = {}
    for kind, table in (('media', 'export_run_items'), ('xmp', 'export_sidecar_items')):
        rows = conn.execute(
            f'SELECT asset_id,dest_relative_path,state,COALESCE(error_code,\'\'),copied_bytes,COALESCE(verified_sha256,\'\') FROM {table} WHERE job_id=?',
            (job,),
        )
        for asset, name, state, error, size, sha in rows:
            report = reports[(kind, name)]
            assert (report['assetId'], report['result'], report['errorCode'], report['sha256']) == (asset, state, error, sha)
            actual[(kind, name)] = (state, error)
            if state == 'copied':
                assert size == (output / name).stat().st_size and sha == digest(output / name)
                if kind == 'media':
                    assert sha == digest(source / name)
                else:
                    ET.parse(output / name)
            else:
                assert size == 0 and not sha
    assert actual == expected and reports.keys() == expected.keys(), actual
    sentinels = {} if same_stamp else SENTINELS
    assert {path.name for path in output.iterdir()} == {manifest.name, *sentinels, *(name for _, name in copied)}
    assert all((output / name).read_bytes() == value for name, value in sentinels.items())
    modified = reuse.source_state()
    assert [row for row in modified if row['name'] != changed[0].name] == [
        row for row in b.STATE['sourceBefore'] if row['name'] != changed[0].name
    ]
    if same_stamp:
        assert changed[0].read_bytes() == (source / 'owned-3.png').read_bytes()
        assert changed[0].read_bytes() != changed[1]
        exported = output / changed[0].name
        b.event('sameStampExportOutcome', changedItemAccepted=exported.exists(),
                exportedSha256=digest(exported) if exported.exists() else None,
                mediaCopied=sum(kind == 'media' for kind, _ in copied),
                xmpCopied=sum(kind == 'xmp' for kind, _ in copied), csvRows=len(reports), partialFiles=0,
                itemResults=[dict(type=kind, filename=name, status=status, code=code)
                             for (kind, name), (status, code) in sorted(actual.items())])
    else:
        assert changed[0].read_bytes() == changed[1] + b'changed after preview'
    restore_source()
    assert reuse.source_state() == b.STATE['sourceBefore']
    if same_stamp:
        b.event('ownedSourcesRestored', sourceCount=len(MEDIA))
        return
    b.event('exportVerified', csvRows=len(reports), mediaCopied=3, mediaFailed=2,
            xmpCopied=2, xmpFailed=1, xmpSkipped=2, partialFiles=0,
            itemResults=[dict(type=kind, filename=name, status=status, code=code)
                         for (kind, name), (status, code) in sorted(actual.items())],
            destinationConflictsPreserved=True, sourceRestored=True)


def snapshot(label):
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    fixture = (b.WORK / 'fixture').resolve(strict=True)
    source, output = fixture / 'source', fixture / 'output'
    assert reuse.normal_state() == b.STATE['normalBefore']
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        assert conn.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not conn.execute('PRAGMA foreign_key_check').fetchall()
        if label == 'before-import':
            assert not conn.execute('SELECT 1 FROM source_roots').fetchone()
            assert not conn.execute('SELECT 1 FROM media_assets').fetchone()
            b.STATE['verifiedPid'] = pid
            b.event('emptyIsolatedCatalogVerified')
        else:
            if label == 'after-bootstrap' and same_stamp:
                assert pid != b.STATE['verifiedPid']
                assert conn.execute('PRAGMA user_version').fetchone() == (10,)
                assert conn.execute('SELECT absolute_path FROM source_roots').fetchall() == [(str(source),)]
                assert {name for (name,) in conn.execute('SELECT relative_path FROM media_assets')} == MEDIA
                assert conn.execute("SELECT 1 FROM jobs WHERE type='SCAN' AND status='completed' AND progress_done=5 AND failure_count=0").fetchone()
                b.STATE['verifiedPid'] = pid
                b.event('controlledSourceScannedNatively', sourceCount=len(MEDIA))
                return
            assert pid == b.STATE['verifiedPid']
            assert conn.execute('SELECT absolute_path FROM source_roots').fetchall() == [(str(source),)]
            assert {name for (name,) in conn.execute('SELECT relative_path FROM media_assets')} == MEDIA
            if label == 'arm-race':
                arm_race(conn, source, output)
            elif label == 'after-export':
                verify_export(conn, source, output)
            else:
                raise ValueError(f'Unknown checkpoint: {label}')


reuse.write_sources, reuse.snapshot = write_sources, snapshot

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--same-stamp', action='store_true', help='Only rewrite one source with unchanged size and modification time')
    same_stamp = parser.parse_args().same_stamp
    if same_stamp:
        b.APPROVAL = reuse.ROOT / 'docs/native-export-same-stamp-retry-default-budget.json'
        b.RECEIPT = reuse.ROOT / 'docs/native-export-same-stamp-retry-latest.json'
        reuse.SCOPE = 'Native export after same-size same-mtime source rewrite following preview'
        reuse.RUN_PREFIX = 'travel-gallery-native-export-same-stamp-'
        reuse.SANDBOX_SEAL_ARGS = ('--webkit-mach-lookup',)
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
