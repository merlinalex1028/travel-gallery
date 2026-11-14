"""Capture a same-size source rewrite after a real native export has written a partial copy."""
from contextlib import closing, suppress
import csv
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
import time

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'export_preview', Path(__file__).with_name('native-export-after-preview-batch.py'))
export = importlib.util.module_from_spec(spec)
spec.loader.exec_module(export)
b, reuse, picker = export.b, export.reuse, export.picker
NAME = 'owned-0.png'
SOURCE_BYTES = 128 * 1024**2
EARLY = 4096
LATE = SOURCE_BYTES - 4096
REWRITE = ((EARLY, b'A'), (LATE, b'B'))


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def write_sources(source):
    export.write_sources(source)
    with (source / NAME).open('r+b') as stream:
        stream.truncate(SOURCE_BYTES)


def rewrite_during_copy(pid, source, output, app, timeout=75):
    original = source.stat()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for part in output.glob(f'.{NAME}.travelgallery-part-*'):
            if part.is_symlink():
                raise RuntimeError('The staged copy must be a regular owned file')
            try:
                size = part.stat().st_size
            except FileNotFoundError:
                continue
            if not EARLY < size < LATE:
                continue
            try:
                os.kill(pid, signal.SIGSTOP)
                status = subprocess.check_output(['ps', '-p', str(pid), '-o', 'state=,comm='], text=True)
                state, path = status.strip().split(None, 1)
                assert state.startswith('T') and path == str(app.resolve(strict=True))
                captured = part.stat().st_size
                if not EARLY < captured < LATE:
                    raise RuntimeError(f'The observed copy finished before the process could be stopped ({captured} bytes)')
                with part.open('rb') as stream:
                    stream.seek(EARLY)
                    assert stream.read(1) == b'\0'
                with source.open('r+b') as stream:
                    for offset, value in REWRITE:
                        stream.seek(offset)
                        assert stream.read(1) == b'\0'
                        stream.seek(offset)
                        stream.write(value)
                    stream.flush()
                    os.fsync(stream.fileno())
                os.utime(source, ns=(original.st_atime_ns, original.st_mtime_ns))
                changed = source.stat()
                assert (changed.st_size, changed.st_ino, changed.st_mtime_ns) == (
                    original.st_size, original.st_ino, original.st_mtime_ns)
                assert changed.st_ctime_ns != original.st_ctime_ns
                return dict(partialBytes=captured, sourceBytes=SOURCE_BYTES, earlyOffset=EARLY, lateOffset=LATE,
                            unchangedSizeMtimeAndInode=True, changeTimeAdvanced=True)
            finally:
                with suppress(ProcessLookupError):
                    os.kill(pid, signal.SIGCONT)
        time.sleep(.001)
    raise TimeoutError('The native worker did not produce an observed incomplete staging file')


def verify_result(conn, source, output, expected_failures=None):
    assert b.STATE['duringCopy']
    jobs = conn.execute("""SELECT id,status,progress_done,progress_total,failure_count FROM jobs
        WHERE type='EXPORT' AND json_extract(checkpoint_json,'$.destination')=?""", (str(output),)).fetchall()
    assert len(jobs) == 1 and jobs[0][1:4] == ('completed', 5, 5), jobs
    job, _, _, _, failures = jobs[0]
    assert failures in (0, 1)
    if expected_failures is not None:
        assert failures == expected_failures
    manifest = output / f'travel-gallery-{job}.csv'
    with manifest.open(encoding='utf-8-sig', newline='') as stream:
        csv_rows = {(row['type'], row['filename']): row for row in csv.DictReader(stream)}
    assert len(csv_rows) == 10
    results = {}
    for kind, table in (('media', 'export_run_items'), ('xmp', 'export_sidecar_items')):
        rows = conn.execute(f"""
            SELECT asset_id,dest_relative_path,state,COALESCE(error_code,''),copied_bytes,
                   COALESCE(verified_sha256,'') FROM {table} WHERE job_id=?
        """, (job,))
        for aid, name, state, error, size, sha in rows:
            expected = ('copied', '')
            if failures and name == NAME:
                expected = ('failed', 'DB_CONFLICT')
            elif failures and name == Path(NAME).with_suffix('.xmp').name:
                expected = ('skipped', 'MEDIA_EXPORT_FAILED')
            assert (state, error) == expected
            row = csv_rows[kind, name]
            assert (row['assetId'], row['result'], row['errorCode'], row['sha256']) == (aid, state, error, sha)
            if state == 'copied':
                path = output / name
                assert path.stat().st_size == size and digest(path) == sha
                if kind == 'media' and name != NAME:
                    assert digest(source / name) == sha
            else:
                assert size == 0 and not sha and not (output / name).exists()
            results[kind, name] = (state, error)
    assert results.keys() == csv_rows.keys()
    assert {p.name for p in output.iterdir()} == {
        manifest.name, *(name for (_, name), (state, _) in results.items() if state == 'copied')
    }
    changed = source / NAME
    original = next(row for row in b.STATE['sourceBefore'] if row['name'] == NAME)
    current_hash = digest(changed)
    assert original['sha256'] != current_hash
    with changed.open('rb') as stream:
        for offset, value in REWRITE:
            stream.seek(offset)
            assert stream.read(1) == value
    copied_hash = None
    if not failures:
        copied = output / NAME
        with copied.open('rb') as stream:
            stream.seek(EARLY)
            assert stream.read(1) == b'\0'
            stream.seek(LATE)
            assert stream.read(1) == REWRITE[1][1]
        copied_hash = digest(copied)
        assert copied_hash not in {original['sha256'], current_hash}
    with changed.open('r+b') as stream:
        for offset, _ in REWRITE:
            stream.seek(offset)
            stream.write(b'\0')
    os.utime(changed, ns=(changed.stat().st_atime_ns, original['mtimeNs']))
    assert reuse.source_state() == b.STATE['sourceBefore']
    b.event('duringCopyOutcome', mixedBytesPublished=not failures, originalSha256=original['sha256'],
            editedSha256=current_hash, copiedSha256=copied_hash, csvRows=len(csv_rows),
            mediaCopied=5-failures, xmpCopied=5-failures, failedItems=failures, ownedSourcesRestored=True)


def snapshot(label):
    if label in ('before-import', 'after-bootstrap'):
        export.snapshot(label)
        return
    database = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(database)
    assert pid == b.STATE['verifiedPid'] and reuse.normal_state() == b.STATE['normalBefore']
    fixture = (b.WORK / 'fixture').resolve(strict=True)
    source, output = fixture / 'source', fixture / 'output'
    with closing(sqlite3.connect(database.as_uri()+'?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        if label == 'after-copy':
            verify_result(conn, source, output)
            return
        assert label == 'arm-copy' and not any(output.iterdir())
        plans = conn.execute('SELECT plan_json,consumed_by_job_id FROM export_plans').fetchall()
        assert len(plans) == 1 and plans[0][1] is None
        plan = json.loads(plans[0][0])
        assert Path(plan['destination']) == output and plan['policy'] == 'rename'
        assert len(plan['items']) == 5 and all(item['xmp'] and not item['skip'] for item in plan['items'])
        item = next(item for item in plan['items'] if item['relative'] == NAME)
        assert item['size'] == SOURCE_BYTES and item['mtime'] == (source / NAME).stat().st_mtime_ns // 1_000_000
    assert reuse.source_state() == b.STATE['sourceBefore']
    b.event('copyRaceArmed', sourceBytes=SOURCE_BYTES)
    capture = rewrite_during_copy(pid, source / NAME, output, b.APP_PATH / 'Contents/MacOS/travel-gallery')
    b.STATE['duringCopy'] = capture
    b.event('sameStampRewriteDuringCopy', **capture)


if __name__ == '__main__':
    b.APPROVAL = reuse.ROOT / 'docs/native-export-during-copy-default-budget.json'
    b.RECEIPT = reuse.ROOT / 'docs/native-export-during-copy-latest.json'
    reuse.SCOPE = 'Native export with an observed incomplete staged copy and same-stamp source rewrite'
    reuse.RUN_PREFIX = 'travel-gallery-native-during-copy-'
    reuse.PLANNED_BYTES = 768 * 1024**2
    reuse.SANDBOX_SEAL_ARGS = ('--webkit-mach-lookup',)
    export.same_stamp = True
    reuse.write_sources, reuse.snapshot = write_sources, snapshot
    timer = Timer(900, lambda: os.kill(os.getpid(), signal.SIGTERM))
    timer.daemon = True
    timer.start()
    try:
        raise SystemExit(reuse.run())
    finally:
        timer.cancel()
