"""Explicit macOS/APFS process-fault acceptance; operates only in a NEW owned directory.

Exercises production Catalog commands in an independent Rust process. It does not
drive a WebView, modify normal app data, or represent a physical power-loss test.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import runpy
import sqlite3
import subprocess
import sys
import time


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, ensure_ascii=False, indent=2)
        stream.write('\n')


def connect(path):
    return sqlite3.connect(path.as_uri() + '?mode=ro', uri=True, timeout=10)


def snapshot(database):
    with connect(database) as db:
        tables = ['asset_reviews', 'tags', 'asset_tags', 'collections', 'collection_assets']
        return dict(
            assets=db.execute('SELECT id,source_id,relative_path,byte_size,mtime_ms FROM media_assets ORDER BY id').fetchall(),
            human={table: db.execute(f'SELECT * FROM {table} ORDER BY rowid').fetchall() for table in tables},
        )


def verify_database(database, baseline):
    assert snapshot(database) == baseline, 'Media identity or human state changed'
    with connect(database) as db:
        assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        assert not db.execute('PRAGMA foreign_key_check').fetchall()
        states = dict(db.execute('SELECT file_state,count(*) FROM media_assets GROUP BY file_state'))
        assert states == {'available': 10000}, states
        return states


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--from-run', type=Path, required=True)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    assert sys.platform == 'darwin', 'This fixture copier requires macOS/APFS'
    original = args.from_run.resolve(strict=True)
    worker = args.worker.resolve(strict=True)
    output = args.output.absolute()
    assert not output.exists() and output.parent.is_dir() and not worker.is_symlink()
    report = json.loads((original / 'report.json').read_text())
    owned = json.loads((original / 'OWNED_BENCHMARK.json').read_text())
    assert report['files'] == owned['count'] and report['files'] >= 10000
    assert report['sourceSha256AndSizeAndNanosecondMtimeUnchanged'] is True
    assert report['humanReviewChanges'] == 0
    clone = runpy.run_path(str(Path(__file__).with_name('prepare-native-performance.py')))['clone']
    output.mkdir(mode=0o700)
    source = output / 'source'
    source.mkdir()
    save(output / 'OWNED_SCAN_FAULTS.json', dict(
        identifier='com.travelgallery.scanfault.qa', count=10000, root=str(output),
        createdAtUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(), fromRun=str(original),
        worker=str(worker), workerSha256=hashlib.sha256(worker.read_bytes()).hexdigest(),
    ))
    manifest = []
    with connect(original / 'data/catalog.sqlite') as db:
        roots = dict(db.execute('SELECT id,absolute_path FROM source_roots'))
        for sid, relative, size, mtime, digest in db.execute(
            'SELECT source_id,relative_path,byte_size,mtime_ms,sha256 FROM media_assets ORDER BY filename,id LIMIT 10000'
        ):
            before = Path(roots[sid]) / relative
            after = source / relative
            assert not Path(relative).is_absolute() and '..' not in Path(relative).parts
            assert before.resolve(strict=True).is_relative_to(Path(roots[sid]).resolve(strict=True))
            assert digest in owned['inputSha256']
            clone(before, after)
            stamp = after.stat()
            assert stamp.st_size == size and stamp.st_mtime_ns // 1000000 == mtime
            manifest.append(dict(path=relative, bytes=size, mtimeNs=stamp.st_mtime_ns, sha256=digest))
    assert len(manifest) == 10000
    save(output / 'source-manifest.json', manifest)

    def run(mode):
        with (output / f'{mode}.log').open('x') as log:
            subprocess.run([str(worker), str(output), mode], stdout=log, stderr=subprocess.STDOUT,
                           check=True, timeout=240)

    database = output / 'data/catalog.sqlite'
    run('prepare')
    baseline = snapshot(database)
    assert len(baseline['assets']) == 10000
    save(output / 'business-baseline.json', baseline)
    verify_database(database, baseline)
    sentinel = source / manifest[-1]['path']
    stash = output / 'held-source.jpg'
    controls = []
    try:
        for mode in ['cancel', 'crash']:
            before = sentinel.stat()
            sentinel.rename(stash)
            assert stash.stat().st_mtime_ns == before.st_mtime_ns
            control = dict(mode=mode, movedFrom=str(sentinel), heldAt=str(stash),
                           sourceMoveByExternalHarness=True, sourceFileMtimeNs=before.st_mtime_ns)
            if mode == 'cancel':
                run(mode)
                result = json.loads((output / 'cancel-result.json').read_text())
                assert 3000 <= result['seenAtRequest'] <= 3500, result
                assert result['job']['status'] == 'cancelled'
                control['atRequest'] = result
            else:
                with (output / 'crash.log').open('x') as log:
                    child = subprocess.Popen([str(worker), str(output), mode], stdout=log, stderr=subprocess.STDOUT)
                    try:
                        started = time.monotonic()
                        while True:
                            assert child.poll() is None, 'Worker exited before rescan started'
                            assert time.monotonic() - started < 30
                            try:
                                launch = json.loads((output / 'crash-start.json').read_text())
                                break
                            except (FileNotFoundError, json.JSONDecodeError):
                                pass
                            time.sleep(.01)
                        assert launch['pid'] == child.pid
                        with connect(database) as db:
                            while True:
                                assert child.poll() is None, 'Worker exited before interruption'
                                seen = db.execute('SELECT count(*) FROM media_assets WHERE source_id=? AND last_seen_scan_generation>?',
                                                  (launch['sourceId'], launch['oldGeneration'])).fetchone()[0]
                                if seen >= 3000:
                                    assert seen <= 3500, f'Interrupted too late: {seen}'
                                    child.kill()  # Only the owned Popen child, never a historic PID.
                                    return_code = child.wait(timeout=10)
                                    assert return_code == -9, return_code
                                    control.update(pid=child.pid, signal='SIGKILL', returnCode=return_code,
                                                   seenBeforeKill=seen, previousJobId=launch['jobId'])
                                    break
                                assert time.monotonic() - started < 120
                                time.sleep(.01)
                    finally:
                        if child.poll() is None:
                            child.kill()
                            child.wait(timeout=10)
            # Missing sentinel is still in the old index: incomplete enumeration
            # must never commit the final missing classification.
            control['statesAfterInterruption'] = verify_database(database, baseline)
            with connect(database) as db:
                start = json.loads((output / f'{mode}-start.json').read_text())
                control['seenAfterInterruption'] = db.execute(
                    'SELECT count(*) FROM media_assets WHERE source_id=? AND last_seen_scan_generation>?',
                    (start['sourceId'], start['oldGeneration'])).fetchone()[0]
            assert 3000 <= control['seenAfterInterruption'] <= 3500, control
            stash.rename(sentinel)
            run('retry-' + mode)
            retry = json.loads((output / f'retry-{mode}-result.json').read_text())
            assert retry['newJob']['status'] == 'completed' and retry['newJob']['progressDone'] == 10000
            if mode == 'crash':
                assert retry['oldJobOnOpen']['status'] == 'failed'
                assert retry['oldJobOnOpen']['errorSummary'] == '上次运行被中断，可安全重试'
            else:
                assert retry['oldJobOnOpen']['status'] == 'cancelled'
            control['retry'] = retry
            control['statesAfterRetry'] = verify_database(database, baseline)
            controls.append(control)
            save(output / f'{mode}-verification.json', control)
    finally:
        if stash.exists() and not sentinel.exists():
            stash.rename(sentinel)

    byte_count = 0
    for expected in manifest:
        path = source / expected['path']
        stamp = path.stat()
        assert (stamp.st_size, stamp.st_mtime_ns) == (expected['bytes'], expected['mtimeNs'])
        with path.open('rb') as stream:
            assert hashlib.file_digest(stream, 'sha256').hexdigest() == expected['sha256']
        assert path.stat().st_mtime_ns == stamp.st_mtime_ns
        byte_count += stamp.st_size
    result = dict(
        atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(), count=10000, sourceBytes=byte_count,
        sourceSha256SizeNanosecondMtimeUnchanged=True, stableIdsAndHumanRowsUnchanged=True,
        sentinelNeverPrematurelyMissing=True, integrityAndForeignKeys='ok', controls=controls,
        scope='Actual production Catalog engine in separate native Rust processes, cooperative cancel and SIGKILL around30%; not WebView GUI, physical power loss or an offline network audit.',
    )
    save(output / 'report.json', result)
    print(json.dumps({k: v for k, v in result.items() if k != 'controls'}, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(f'Scan interruption acceptance failed: {error}', file=sys.stderr)
        raise
