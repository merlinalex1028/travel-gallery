"""Run the approved exact-file-version regression, native upgrade, and delivery batch."""
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
spec = importlib.util.spec_from_file_location('build_batch', Path(__file__).with_name('source-reauthorization-build-batch.py'))
batch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(batch)
b, reuse, picker, sha = batch.b, batch.reuse, batch.picker, batch.sha
ROOT, LIMITS, BUDGET = batch.ROOT, batch.LIMITS, batch.BUDGET
BATCH = 'exact-file-version'
b.APPROVAL = ROOT / 'docs/native-exact-file-version-budget.json'
b.RECEIPT = ROOT / 'docs/native-exact-file-version-latest.json'
EXECUTABLE = Path('Contents/MacOS/travel-gallery')
NATIVE_COUNT = 127
DUPLICATES = {'owned-0.png', 'owned-1.png', 'owned-2.png', *(f'owned-{i:03}.png' for i in range(5, NATIVE_COUNT))}
STEPS = ('empty', 'legacy', 'upgrade', 'migrated', 'rewrite-direct', 'direct',
         'rewrite-rescan', 'rescanned', 'manual', 'finish')


def check_release(info, app):
    archive = ROOT / 'artifacts' / info['file']
    assert sha(archive) == info['sha256'] and sha(app / EXECUTABLE) == info['executableSha256']
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    subprocess.run(['/usr/bin/unzip', '-tq', str(archive)], check=True)


def seal_for_qa(app, label, fixture, env):
    unsigned = b.WORK / f'unsigned-{label}' / 'Travel Gallery Offline Bench.app'
    unsigned.parent.mkdir()
    b.command('copyForQA' + label, ['ditto', str(app), str(unsigned)], env)
    assert sha(unsigned / EXECUTABLE) == sha(app / EXECUTABLE)
    plist = unsigned / 'Contents/Info.plist'
    info = plistlib.loads(plist.read_bytes())
    info.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName=unsigned.stem, CFBundleDisplayName=unsigned.stem)
    plist.write_bytes(plistlib.dumps(info))
    output = b.WORK / f'qa-{label}'
    b.command('sealForQA' + label, [sys.executable, 'scripts/prepare-offline-sandbox.py',
              '--source', str(unsigned), '--fixture', str(fixture), '--output', str(output),
              '--webkit-mach-lookup'], env)
    shutil.rmtree(unsigned.parent)
    return output / unsigned.name


def exited(app):
    command = str((app / EXECUTABLE).resolve())
    processes = subprocess.check_output(['ps', '-axww', '-o', 'comm='], text=True)
    return command not in {row.strip() for row in processes.splitlines()}


def database_snapshot():
    path = b.DATA / 'catalog.sqlite'
    pid = picker.verified_process(path)
    with closing(sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)) as db:
        db.execute('BEGIN')
        assert db.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not db.execute('PRAGMA foreign_key_check').fetchall()
        assert not db.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
        version = db.execute('PRAGMA user_version').fetchone()[0]
        roots = db.execute('SELECT id,absolute_path,is_online,is_enabled FROM source_roots').fetchall()
        version_columns = ',file_version,sha256_file_version' if version == 10 else ''
        assets = db.execute('SELECT id,relative_path,sha256,byte_size,mtime_ms' + version_columns
                            + ' FROM media_assets ORDER BY id').fetchall()
        reviews = db.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        marks = db.execute("SELECT COUNT(*) FROM asset_reviews WHERE rating=4 AND pick_state='keep'").fetchone()[0]
        groups = {}
        decisions = {}
        for gid, representative, name, excluded in db.execute("""
            SELECT g.id,g.representative_asset_id,a.relative_path,i.manually_excluded
            FROM similar_groups g JOIN similar_group_items i ON i.group_id=g.id
            JOIN media_assets a ON a.id=i.asset_id WHERE g.group_type='exact' ORDER BY g.id,a.id
        """):
            groups.setdefault(gid, []).append(name)
            decisions.setdefault(gid, [representative, []])
            if excluded:
                decisions[gid][1].append(name)
        cache = dict(db.execute("SELECT asset_id,cache_rel_path FROM thumbnail_cache WHERE variant='small'"))
        scans = db.execute("SELECT status,progress_done,failure_count FROM jobs WHERE type='SCAN' ORDER BY created_at,rowid").fetchall()
        hashes = db.execute("""SELECT COALESCE(json_extract(checkpoint_json,'$.operation'),'manual'),
            status,failure_count FROM jobs WHERE type='HASH_EXACT' ORDER BY created_at,rowid""").fetchall()
    return dict(pid=pid, version=version, roots=roots, assets=assets, reviews=reviews, marks=marks, groups=groups,
                decisions=decisions, cache=cache, scans=scans, hashes=hashes)


def identity(state):
    rows = [state['roots'][0][:2], [asset[:2] for asset in state['assets']], state['reviews']]
    return hashlib.sha256(json.dumps(rows).encode()).hexdigest()


def groups_match(state, expected):
    assert {frozenset(group) for group in state['groups'].values()} == {frozenset(group) for group in expected}


def hashes_match(state, source, names):
    for _, name, digest, size, mtime, indexed, hashed in state['assets']:
        if name in names:
            path = source / name
            assert digest == sha(path) and indexed and indexed == hashed
            assert size == path.stat().st_size and mtime == path.stat().st_mtime_ns // 1_000_000


def rewrite(path, donor):
    before = path.stat()
    bytes_before = path.read_bytes()
    bytes_after = donor.read_bytes()
    assert bytes_before != bytes_after and len(bytes_before) == len(bytes_after)
    path.write_bytes(bytes_after)
    os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
    after = path.stat()
    assert (before.st_size,before.st_ino,before.st_mtime_ns) == (after.st_size,after.st_ino,after.st_mtime_ns)
    assert before.st_ctime_ns != after.st_ctime_ns
    return path, bytes_before, before.st_atime_ns, before.st_mtime_ns


def publish(candidate, old_release, env):
    # Keep a single owned rollback in the active batch until both outputs verify.
    rollback = b.WORK / 'rollback'
    rollback.mkdir()
    files = [old_release['file'], 'build-info.json', 'SHA256SUMS.txt']
    for name in files:
        shutil.copy2(ROOT / 'artifacts' / name, rollback / name)
    old_app = rollback / b.NORMAL.name
    installed = False
    try:
        b.command('packageVerifiedCandidate', ['node', 'scripts/package-macos.mjs'],
                  dict(env, TRAVEL_GALLERY_PACKAGE_APP=str(candidate)))
        new_release = json.loads((ROOT / 'artifacts/build-info.json').read_text())
        assert new_release['executableSha256'] == b.STATE['candidateExecutableSha256']
        check_release(new_release, candidate)
        b.NORMAL.rename(old_app)
        candidate.rename(b.NORMAL)
        check_release(new_release, b.NORMAL)
        b.STATE['release'] = new_release
        installed = True
    finally:
        if not installed:
            if old_app.exists():
                if b.NORMAL.exists():
                    b.NORMAL.rename(candidate)
                old_app.rename(b.NORMAL)
            for name in files:
                (rollback / name).replace(ROOT / 'artifacts' / name)
            check_release(old_release, b.NORMAL)


def native_steps(candidate, legacy, upgraded, source, release, env):
    initial = reuse.source_state()
    originals = []
    control = b.WORK / 'control'
    control.mkdir()
    baseline = None
    remaining = set(DUPLICATES)
    b.STATE.update(status='ready', application=str(legacy), controlDirectory=str(control),
                   expectedDatabase=str(b.DATA / 'catalog.sqlite'))
    b.event('legacyAppReady', source=str(source), **b.check())
    try:
        for step in STEPS:
            deadline = time.monotonic() + 600
            request = control / 'step.json'
            while not request.is_file():
                b.check()
                if time.monotonic() >= deadline:
                    raise TimeoutError(f'No native checkpoint: {step}')
                time.sleep(.25)
            assert json.loads(request.read_text()) == {'step': step}
            request.unlink()
            assert reuse.normal_state() == b.STATE['normalBefore']
            if step == 'upgrade':
                assert exited(legacy)
                b.APP_PATH = upgraded
                b.STATE['application'] = str(upgraded)
                b.event('candidateAppReady', source=str(source), **b.check())
                continue
            if step == 'finish':
                assert exited(upgraded) and exited(legacy)
                assert reuse.source_state() == initial
                assert batch.verify_inputs() == b.STATE['sourceCommit']
                publish(candidate, release, env)
                b.STATE.update(status='finished', qaQuitNormally=True)
                return
            state = database_snapshot()
            if step == 'empty':
                assert state['version'] == 9 and not state['roots'] and not state['assets']
                b.event('legacyIsolationVerified')
                continue
            assert len(state['assets']) == len(state['reviews']) == NATIVE_COUNT and state['marks'] == 1
            assert len(state['roots']) == 1 and state['roots'][0][1:] == (str(source), 1, 0)
            if step == 'legacy':
                assert state['version'] == 9 and state['hashes'] == [('manual','completed',0)]
                assert state['scans'] and all(scan == ('completed',NATIVE_COUNT,0) for scan in state['scans'])
                assert len(state['cache']) == NATIVE_COUNT
                groups_match(state, [DUPLICATES])
                representative, excluded = next(iter(state['decisions'].values()))
                assert representative and len(excluded) == 1
                baseline = state
                b.event('legacyCatalogVerified', members=len(DUPLICATES), assets=NATIVE_COUNT, decisionsPreserved=True)
                continue
            assert baseline and identity(state) == identity(baseline) and state['pid'] != baseline['pid']
            assert state['version'] == 10
            backups = list(b.DATA.glob('catalog.pre-migration-*.sqlite'))
            assert len(backups) == 1
            with closing(sqlite3.connect(backups[0].as_uri() + '?mode=ro', uri=True)) as backup:
                assert backup.execute('PRAGMA user_version').fetchone() == (9,)
            if step == 'migrated':
                groups_match(state, [DUPLICATES])
                assert state['hashes'] == baseline['hashes'] + [('recomputeExact','completed',0)]
                assert list(state['decisions'].values()) == list(baseline['decisions'].values())
                assert state['cache'] == baseline['cache']
                hashes_match(state, source, DUPLICATES)
                b.event('nativeUpgradeVerified', beforeVersion=9, afterVersion=10, migrationBackup=True,
                        members=len(DUPLICATES), originalCachesPreserved=len(state['cache']))
            elif step.startswith('rewrite'):
                groups_match(state, [remaining])
                name = next(name for _, name, *_ in reversed(state['assets']) if name in remaining)
                remaining.remove(name)
                donor = source / ('owned-3.png' if step == 'rewrite-direct' else 'owned-4.png')
                originals.append(rewrite(source / name, donor))
                b.event('sameStampRewrite', mode=step, name=name, unchangedSizeMtimeAndInode=True)
            elif step in ('direct', 'rescanned'):
                round_number = 1 if step == 'direct' else 2
                groups_match(state, [remaining])
                assert state['hashes'] == baseline['hashes'] + [('recomputeExact','completed',0)] * (round_number + 1)
                assert len(state['scans']) == len(baseline['scans']) + (step == 'rescanned')
                if step == 'rescanned':
                    assert state['scans'][-1] == ('completed',NATIVE_COUNT,0)
                hashes_match(state, source, DUPLICATES)
                for aid, name, *_ in state['assets']:
                    if name in remaining:
                        assert state['cache'][aid] == baseline['cache'][aid]
                    if any(path.name == name for path, *_ in originals):
                        assert state['cache'].get(aid) != baseline['cache'][aid]
                b.event('automaticRecomputeVerified', mode=step, oldGroupMembers=len(remaining),
                        automaticJobs=round_number, sourceIdsAndReviewsPreserved=True)
            elif step == 'manual':
                groups_match(state, [remaining, {originals[0][0].name, 'owned-3.png'},
                                     {originals[1][0].name, 'owned-4.png'}])
                assert state['hashes'] == baseline['hashes'] + [('recomputeExact','completed',0)] * 3 + [('manual','completed',0)]
                hashes_match(state, source, {asset[1] for asset in state['assets']})
                for path, content, atime, mtime in originals:
                    path.write_bytes(content)
                    os.utime(path, ns=(atime, mtime))
                originals.clear()
                assert reuse.source_state() == initial
                b.event('manualRehashVerified', groups=[len(remaining),2,2], sourceRestored=True)
    finally:
        for path, content, atime, mtime in originals:
            path.write_bytes(content)
            os.utime(path, ns=(atime, mtime))


def run(approval):
    assert not any(path.exists() or path.is_symlink() for path in [*reuse.OWNED, *b.EXTRA, b.GENERATED])
    assert shutil.disk_usage(ROOT).free >= approval['requiredFreeBytes']
    subprocess.run(BUDGET + ['--approval-file', str(b.APPROVAL), '--planned-files', str(LIMITS['files']),
                            '--planned-bytes', str(LIMITS['logicalBytes']), '--planned-source-files',
                            str(LIMITS['sourceFiles'])], check=True)
    release = json.loads((ROOT / 'artifacts/build-info.json').read_text())
    assert release['sha256'].startswith('2807fa03')
    check_release(release, b.NORMAL)
    b.STATE.update(sourceCommit=batch.verify_inputs(), normalBefore=reuse.normal_state(),
                   acceptedReleaseBefore=release['sha256'])
    b.TEMP = tempfile.TemporaryDirectory(prefix='travel-gallery-exact-version-')
    b.WORK = Path(b.TEMP.name).resolve()
    b.OWNS_GENERATED = True
    b.STATE['temporaryRoot'] = str(b.WORK)
    b.save()
    cases = b.WORK / 'cases'
    cases.mkdir()
    env = b.prepare_home()
    env.update(TMPDIR=str(cases), CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
    b.command('fullRustRegressions', ['node','scripts/native.mjs','cargo','test','--locked',
              '--manifest-path','src-tauri/Cargo.toml','--','--test-threads=1'], env)
    assert not any(cases.iterdir())
    shutil.rmtree(b.WORK / 'target')
    candidate = b.build(False, env)
    b.STATE.update(normalCandidate=str(candidate), candidateExecutableSha256=sha(candidate / EXECUTABLE))
    fixture = b.WORK / 'fixture'
    source = fixture / 'source'
    source.mkdir(parents=True)
    reuse.write_sources(source)
    original = (source / 'owned-0.png').read_bytes()
    for name in DUPLICATES:
        if name != 'owned-0.png':
            (source / name).write_bytes(original)
    assert len(list(source.iterdir())) == NATIVE_COUNT
    marker = dict(identifier=b.IDENTIFIER, count=NATIVE_COUNT, fixtureRoot=str(source), scope=BATCH)
    (fixture / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    legacy = seal_for_qa(b.NORMAL, 'Legacy', fixture, env)
    upgraded = seal_for_qa(candidate, 'Candidate', fixture, env)
    b.APP_PATH = legacy
    b.OWNED_PATHS.extend(reuse.OWNED)
    native_steps(candidate, legacy, upgraded, source, release, env)


if __name__ == '__main__':
    batch.run = run
    if not b.APPROVAL.is_file() or b.APPROVAL.is_symlink():
        raise SystemExit('This batch needs its approved budget record')
    with b.APPROVAL.open('r+') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        approval = json.load(handle)
        assert (approval.get('batch') == BATCH and approval.get('status') == 'active'
                and approval.get('userAnswer') == '允许这一轮，结束即清理'
                and not approval.get('claimedAt') and approval['limits'] == LIMITS
                and approval['requiredFreeBytes'] == LIMITS['logicalBytes'])
        approval['claimedAt'] = datetime.now(timezone.utc).isoformat()
        handle.seek(0)
        json.dump(approval, handle, ensure_ascii=False, indent=2)
        handle.truncate()
        handle.flush()
        raise SystemExit(batch.main(approval))
