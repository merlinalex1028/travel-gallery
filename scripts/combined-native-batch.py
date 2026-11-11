"""Run the approved three-fix regression and macOS QA batch in one owned directory."""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
from native_fixture_budget import source_inventory

ROOT = Path(__file__).resolve().parent.parent
APPROVAL = ROOT / 'docs/combined-native-budget.json'
FROZEN = ROOT / 'docs/combined-native-inputs.json'
BATCH = 'query-scroll-redundancy-recompute'
LIMITS = {'files': 40_000, 'logicalBytes': 6 * 1024**3, 'sourceFiles': 500}
NATIVE_COUNT = 125
NATIVE_BYTES = 1024**2

spec = importlib.util.spec_from_file_location('sorting', ROOT / 'scripts/native-sorting-batch.py')
sorting = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sorting)
reuse, b = sorting.reuse, sorting.reuse.b


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def authorized(record):
    return (
        record.get('batch') == BATCH
        and record.get('limits') == LIMITS
        and record.get('requiredFreeBytes') == LIMITS['logicalBytes']
        and record.get('nativeSourceFiles') == NATIVE_COUNT
        and record.get('nativeSourceBytes') == NATIVE_BYTES
        and record.get('status') == 'active'
        and record.get('userAnswer') == '允许这一轮，结束即清理'
        and not record.get('claimedAt')
        and not record.get('closedAt')
    )


def verify_inputs():
    frozen = json.loads(FROZEN.read_text())
    for name, digest in frozen['inputs'].items():
        path = ROOT / name
        if path.is_symlink() or not path.is_file() or sha(path) != digest:
            raise RuntimeError(f'Frozen input differs: {name}')
    control = json.loads((ROOT / 'docs/native-batch-control-check.json').read_text())
    if not (control['exitCode'] == 0 and control['approvalClosed'] and control['allTemporaryDataRemoved']
            and sha(ROOT / 'scripts/exit-confirmation-batch.py') == control['controllerSourceSha256']):
        raise RuntimeError('The reused controller is missing a matching cleanup receipt')
    return len(frozen['inputs'])


def formal_release():
    info = json.loads((ROOT / 'artifacts/build-info.json').read_text())
    archive = ROOT / 'artifacts' / info['file']
    sums = (ROOT / 'artifacts/SHA256SUMS.txt').read_text().split()
    if (archive.is_symlink() or archive.stat().st_size != info['bytes'] or sha(archive) != info['sha256']
            or sums != [info['sha256'], archive.name]):
        raise RuntimeError('The current formal ZIP does not match its receipt')
    subprocess.run(['/usr/bin/unzip', '-tq', str(archive)], check=True, stdout=subprocess.DEVNULL)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(b.NORMAL)], check=True)
    with (b.NORMAL / 'Contents/Info.plist').open('rb') as stream:
        if plistlib.load(stream)['CFBundleIdentifier'] != 'com.travelgallery.desktop':
            raise RuntimeError('The build template must be the formal App')
    for name, field in [('travel-gallery', 'executableSha256'), ('travel-gallery-ai', 'aiWorkerSha256'),
                        ('travel-gallery-quality', 'qualityWorkerSha256'), ('ffmpeg', 'ffmpegSha256'),
                        ('ffprobe', 'ffprobeSha256')]:
        if sha(b.NORMAL / 'Contents/MacOS' / name) != info[field]:
            raise RuntimeError(f'The current formal App differs from its receipt: {name}')
    return info


def write_sources(source):
    for index in range(NATIVE_COUNT):
        capture, offset = ('2024:03:24 08:00:00', None) if index < 121 else {
            121: ('2024:03:23 23:00:00', '+08:00'),
            122: ('2024:03:24 10:00:00', '+08:00'),
        }.get(index, (None, None))
        path = source / f'sort-{index:03}.tiff'
        path.write_bytes(sorting.tiny_tiff(0 if index < 121 else index, capture, offset))
        stamp = datetime(2024, 3, 24 if index != 124 else 22, 8, tzinfo=timezone.utc).timestamp()
        os.utime(path, (stamp, stamp))


def inventory(fixture):
    return {p.name: (sha(p), p.stat().st_size, p.stat().st_mtime_ns, p.stat().st_ino)
            for p in source_inventory(fixture)}


def qa_database(pid):
    executable = b.APP_PATH / 'Contents/MacOS/travel-gallery'
    if type(pid) is not int or pid <= 0:
        raise RuntimeError('A live QA PID is required')
    comm = subprocess.check_output(['ps', '-p', str(pid), '-o', 'comm='], text=True).strip()
    if comm != str(executable.resolve(strict=True)):
        raise RuntimeError('PID is not the batch-owned QA executable')
    open_files = subprocess.check_output(['/usr/sbin/lsof', '-a', '-p', str(pid), '-Fn'], text=True)
    lock = b.DATA / 'catalog.lock'
    database = b.DATA / 'catalog.sqlite'
    if lock.is_symlink() or not lock.is_file() or 'n' + str(lock.resolve()) not in open_files.splitlines():
        raise RuntimeError('QA process is not using the expected isolated catalog lock')
    if database.is_symlink() or not database.is_file():
        raise RuntimeError('The isolated catalog is missing or linked')
    conn = sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)
    conn.execute('BEGIN')
    return conn


def recompute_jobs(conn):
    return conn.execute(
        """SELECT id,status,failure_count,checkpoint_json FROM jobs WHERE type='HASH_EXACT'
           AND json_extract(checkpoint_json,'$.operation')='recomputeExact' ORDER BY created_at,id""",
    ).fetchall()


def replace_late_members(conn, source):
    roots = conn.execute('SELECT absolute_path,is_enabled FROM source_roots').fetchall()
    if roots != [(str(source), 0)]:
        raise RuntimeError('Disable watching on the one owned source before changing fixture files')
    if conn.execute("SELECT count(*) FROM jobs WHERE status IN ('queued','running','paused')").fetchone()[0]:
        raise RuntimeError('Wait until existing QA jobs have stopped')
    groups = conn.execute("SELECT id FROM similar_groups WHERE group_type='exact'").fetchall()
    if len(groups) != 1 or recompute_jobs(conn):
        raise RuntimeError('Expected the first manual exact group and zero automatic attempts')
    gid = groups[0][0]
    members = conn.execute(
        """SELECT a.id,a.relative_path,a.byte_size FROM similar_group_items gi
           JOIN media_assets a ON a.id=gi.asset_id WHERE gi.group_id=? ORDER BY a.id""", (gid,),
    ).fetchall()
    if len(members) != 121:
        raise RuntimeError('Expected the intact 121-member group')
    replacement = sorting.tiny_tiff(210)
    changes = []
    for aid, relative, old_size in members[-2:]:
        path = source / relative
        if path.parent != source or path.is_symlink() or not path.is_file() or len(replacement) == old_size:
            raise RuntimeError('Replacement must target only owned files with a distinct byte size')
        changes.append((aid, path))
    for _, path in changes:
        path.write_bytes(replacement)
    return gid, {aid: path.name for aid, path in changes}


def verify_recompute(conn, source, old_group, changed):
    jobs = recompute_jobs(conn)
    if len(jobs) != 1 or jobs[0][1:3] != ('completed', 0) or json.loads(jobs[0][3])['groupIds'] != [old_group]:
        raise RuntimeError('Expected one successful automatic attempt for the old group')
    rows = conn.execute(
        """SELECT g.id,a.id,a.relative_path,a.byte_size,a.mtime_ms,a.sha256,a.sha256_byte_size,a.sha256_mtime_ms
           FROM similar_groups g JOIN similar_group_items gi ON gi.group_id=g.id
           JOIN media_assets a ON a.id=gi.asset_id WHERE g.group_type='exact'""",
    ).fetchall()
    indexed_groups = {}
    for group, aid, relative, size, mtime, digest, hash_size, hash_mtime in rows:
        path = source / relative
        if path.parent != source or path.is_symlink() or not path.is_file():
            raise RuntimeError('Rebuilt result points outside the owned fixture')
        if (digest, size, mtime) != (sha(path), path.stat().st_size, path.stat().st_mtime_ns // 10**6):
            raise RuntimeError('Rebuilt hash or metadata does not match the current source')
        if (size, mtime) != (hash_size, hash_mtime):
            raise RuntimeError('Rebuilt hash version does not match the refreshed index')
        indexed_groups.setdefault(group, {})[aid] = (relative, digest)
    if old_group in indexed_groups or sorted(map(len, indexed_groups.values())) != [2, 119]:
        raise RuntimeError('Expected the old exact group to split into 119 and 2')
    if any(len({digest for _, digest in members.values()}) != 1 for members in indexed_groups.values()):
        raise RuntimeError('Exact groups must contain identical bytes')
    pair = next(members for members in indexed_groups.values() if len(members) == 2)
    if {aid: relative for aid, (relative, _) in pair.items()} != changed:
        raise RuntimeError('The new size bucket did not contain the two changed members')
    if conn.execute("""SELECT 1 FROM similar_groups g JOIN similar_group_items gi ON gi.group_id=g.id
                       WHERE g.group_type='exact' AND (g.representative_asset_id IS NOT NULL
                         OR gi.manually_excluded<>0) LIMIT 1""").fetchone():
        raise RuntimeError('Group decisions were carried over to a different member set')
    return jobs[0][0]


def human_state(conn):
    return tuple(conn.execute(sql).fetchall() for sql in (
        'SELECT * FROM asset_reviews ORDER BY asset_id',
        'SELECT * FROM selection_snapshot_items ORDER BY snapshot_id,asset_id',
    ))


def control_loop(source, initial):
    control = b.WORK / 'control'
    control.mkdir()
    b.STATE['controlDirectory'] = str(control)
    b.event('controlReady', path=str(control))
    held = source.with_name('held-out')
    pid = None
    old_group, changed = None, {}
    before_human = None
    expected = initial
    while True:
        b.check()
        for path in sorted(control.glob('*.json')):
            request = json.loads(path.read_text())
            path.unlink()
            op = request['op']
            if op == 'finish':
                if pid and subprocess.run(['ps', '-p', str(pid)], stdout=subprocess.DEVNULL).returncode == 0:
                    b.event('finishDeferred', reason='Quit the isolated app before finishing')
                    continue
                b.STATE['status'] = 'finished' if b.STATE.get('recomputeVerified') else 'incomplete'
                return expected
            with closing(qa_database(request['pid'] if op == 'profile' else pid)) as conn:
                if op == 'profile':
                    if any(conn.execute(f'SELECT 1 FROM {table} LIMIT 1').fetchone() for table in ('source_roots', 'media_assets', 'jobs')):
                        raise RuntimeError('Verify the empty QA catalog before import')
                    pid = request['pid']
                    b.event('qaProfileVerified', pid=pid, catalog=str(b.DATA / 'catalog.sqlite'))
                elif op == 'disconnect' and old_group is None and not held.exists():
                    if conn.execute('SELECT absolute_path,is_enabled FROM source_roots').fetchall() != [(str(source), 0)]:
                        raise RuntimeError('Disable watching on the owned source before disconnecting it')
                    source.rename(held)
                    b.event('ownedSourceDisconnected')
                elif op == 'reconnect' and held.is_dir() and not held.is_symlink() and not source.exists():
                    if recompute_jobs(conn):
                        raise RuntimeError('Offline group queries scheduled a recomputation')
                    held.rename(source)
                    b.event('ownedSourceReconnected', autoAttemptsWhileOffline=0)
                elif op == 'replace' and old_group is None and not held.exists():
                    if inventory(source.parent) != initial:
                        raise RuntimeError('Owned source files changed before the controlled replacement')
                    before_human = human_state(conn)
                    old_group, changed = replace_late_members(conn, source)
                    expected = inventory(source.parent)
                    if {name for name in initial if initial[name] != expected[name]} != set(changed.values()):
                        raise RuntimeError('Unexpected source changes during fixture replacement')
                    b.event('lateFixtureMembersReplaced', files=list(changed.values()))
                elif op == 'verify' and old_group is not None:
                    if any(job[1] in ('queued', 'running', 'paused') for job in recompute_jobs(conn)):
                        b.event('verificationDeferred', reason='The recomputation is still active')
                        continue
                    job = verify_recompute(conn, source, old_group, changed)
                    if inventory(source.parent) != expected:
                        raise RuntimeError('Application changed a fixture file')
                    if human_state(conn) != before_human:
                        raise RuntimeError('Recomputation changed a review or frozen selection')
                    b.STATE['recomputeVerified'] = True
                    b.event('recomputeVerified', jobId=job, groupSizes=[119, 2], automaticAttempts=1,
                            reviewsAndSelectionsPreserved=True)
                else:
                    raise RuntimeError(f'Unexpected control operation: {op}')
        time.sleep(1)


def run(approval):
    b.APPROVAL = APPROVAL
    b.RECEIPT = ROOT / 'docs/combined-native-latest.json'
    original_check = b.check
    original_save = b.save

    def check():
        stats = original_check()
        if b.WORK:
            media = {'.png', '.jpg', '.jpeg', '.gif', '.webp', '.tiff', '.tif', '.bmp', '.mov', '.mp4', '.heic', '.raw'}
            cases = sum(p.suffix.lower() in media and p.is_file() for p in (b.WORK / 'cases').rglob('*'))
            native = source_inventory(b.WORK / 'fixture')
            if cases > LIMITS['sourceFiles'] or len(native) > NATIVE_COUNT or sum(p.stat().st_size for p in native) > NATIVE_BYTES:
                raise RuntimeError('Test fixture exceeds the approved source limits')
            b.STATE['peakObservedRustMediaFiles'] = max(b.STATE.get('peakObservedRustMediaFiles', 0), cases)
        return stats

    def save():
        if b.STATE.get('temporaryDirectoryRemoved') and b.WORK:
            for key in ('temporaryRoot', 'controlDirectory', 'normalCandidate', 'application', 'expectedDatabase'):
                b.STATE.pop(key, None)
            for event in b.STATE['events']:
                if event['stage'] == 'qaProfileVerified':
                    event.pop('pid', None)
                    event.pop('catalog', None)
            cleaned = json.dumps(b.STATE, ensure_ascii=False).replace(str(b.WORK), '<cleaned run>')
            b.STATE.update(json.loads(cleaned))
        original_save()

    def main():
        if any(p.exists() or p.is_symlink() for p in [*reuse.OWNED, *b.EXTRA, b.GENERATED]):
            raise RuntimeError('An existing QA namespace or generated schema needs review')
        if shutil.disk_usage(ROOT).free < approval['requiredFreeBytes']:
            raise RuntimeError('Insufficient free space for the approved batch')
        b.STATE['formalBefore'] = reuse.normal_state()
        b.STATE['acceptedReleaseBefore'] = formal_release()['sha256']
        subprocess.run([sys.executable, str(ROOT / 'scripts/check-test-artifact-budget.py'),
                        '--approval-file', str(APPROVAL), '--planned-files', str(LIMITS['files']),
                        '--planned-bytes', str(LIMITS['logicalBytes']), '--planned-source-files',
                        str(LIMITS['sourceFiles'])], check=True)
        b.OWNS_GENERATED = True
        b.TEMP = tempfile.TemporaryDirectory(prefix='travel-gallery-combined-')
        b.WORK = Path(b.TEMP.name).resolve()
        b.STATE['temporaryRoot'] = str(b.WORK)
        b.save()
        cases = b.WORK / 'cases'
        cases.mkdir()
        env = b.prepare_home()
        env.update(TMPDIR=str(cases), CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
        b.command('fullRustRegressions', ['node', 'scripts/native.mjs', 'cargo', 'test', '--locked',
                  '--manifest-path', 'src-tauri/Cargo.toml', '--', '--test-threads=1'], env)
        if any(cases.iterdir()):
            raise RuntimeError('A Rust fixture leaked before the GUI stage')
        shutil.rmtree(b.WORK / 'target')

        fixture = b.WORK / 'fixture'
        source = fixture / 'source'
        source.mkdir(parents=True)
        write_sources(source)
        initial = inventory(fixture)
        marker = dict(identifier=b.IDENTIFIER, count=NATIVE_COUNT, fixtureRoot=str(source), scope=BATCH)
        (fixture / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
        candidate = b.build(False, env)
        b.STATE['normalCandidate'] = str(candidate)
        stage = b.WORK / 'qa-unsigned' / 'Travel Gallery Offline Bench.app'
        stage.parent.mkdir()
        b.command('copyProductionForQA', ['ditto', str(candidate), str(stage)], env)
        executable = Path('Contents/MacOS/travel-gallery')
        digest = sha(stage / executable)
        if digest != sha(candidate / executable):
            raise RuntimeError('QA copy differs from the formal executable before signing')
        b.STATE['qaProductionExecutableBeforeSigning'] = digest
        plist = stage / 'Contents/Info.plist'
        info = plistlib.loads(plist.read_bytes())
        info.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName=stage.stem, CFBundleDisplayName=stage.stem)
        plist.write_bytes(plistlib.dumps(info))
        b.command('sealProductionQA', [sys.executable, 'scripts/prepare-offline-sandbox.py', '--source',
                  str(stage), '--fixture', str(fixture), '--output', str(b.WORK / 'qa'), '--webkit-mach-lookup'], env)
        shutil.rmtree(stage.parent)
        b.APP_PATH = b.WORK / 'qa' / stage.name
        for directory in (b.DATA, b.CACHE):
            directory.mkdir(parents=True)
            b.OWNED_PATHS.append(directory)
            (directory / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
        b.STATE.update(status='ready', application=str(b.APP_PATH), expectedDatabase=str(b.DATA / 'catalog.sqlite'))
        b.event('readyForNativeGui', application=str(b.APP_PATH), source=str(source), **check())
        expected = control_loop(source, initial)
        if inventory(fixture) != expected:
            raise RuntimeError('Fixture inventory differed at QA exit')

    def checked_main():
        try:
            main()
        finally:
            if 'formalBefore' in b.STATE:
                b.STATE['formalAfter'] = reuse.normal_state()
                b.STATE['formalPreserved'] = b.STATE['formalBefore'] == b.STATE['formalAfter']
                if not b.STATE['formalPreserved'] and b.STATE['status'] == 'finished':
                    raise RuntimeError('Formal profile changed during the batch')

    b.check, b.save, b.main = check, save, checked_main
    result = b.run()
    postflight = subprocess.run([sys.executable, str(ROOT / 'scripts/check-test-artifact-budget.py')],
                                check=False, capture_output=True, text=True)
    b.STATE['postCleanupBudget'] = json.loads(postflight.stdout)
    b.save()
    if postflight.returncode or b.STATE['postCleanupBudget']['totalPlannedFiles']:
        raise RuntimeError('Temporary artifacts remain after the batch')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preflight', action='store_true', help='Verify frozen inputs and current release without generating files')
    args = parser.parse_args()
    if args.preflight:
        count = verify_inputs()
        print(json.dumps(dict(frozenInputs=count, formalSha256=formal_release()['sha256'],
                              budgetStatus=json.loads(APPROVAL.read_text())['status'],
                              availableBytes=shutil.disk_usage(ROOT).free)))
    else:
        if APPROVAL.is_symlink():
            raise SystemExit('Approval must be a project-owned file.')
        with APPROVAL.open('r+') as handle:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            record = json.load(handle)
            if not authorized(record):
                raise SystemExit('This batch needs fresh, unclaimed approval; no temporary files generated.')
            verify_inputs()
            record['claimedAt'] = datetime.now(timezone.utc).isoformat()
            handle.seek(0)
            json.dump(record, handle, ensure_ascii=False, indent=2)
            handle.write('\n')
            handle.truncate()
            handle.flush()
            raise SystemExit(run(record))
