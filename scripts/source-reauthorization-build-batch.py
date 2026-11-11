"""Build one macOS candidate and run both original-read and restart checks."""
from datetime import datetime, timezone
import fcntl
import hashlib
import importlib.util
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('picker', ROOT / 'scripts/native-picker-restart-batch.py')
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = ROOT / 'docs/source-reauthorization-budget.json'
b.RECEIPT = ROOT / 'docs/source-reauthorization-latest.json'
LIMITS = dict(files=25_000, logicalBytes=4 * 1024**3, sourceFiles=500)
CHECKPOINTS = ('before-import', 'before-restart', 'after-restart', 'after-denied-rescan', 'after-reselect')
BUDGET = [sys.executable, str(ROOT / 'scripts/check-test-artifact-budget.py')]


def sha(path):
    with path.open('rb') as contents:
        return hashlib.file_digest(contents, 'sha256').hexdigest()


def verify_inputs():
    subprocess.run(['git', 'diff', '--exit-code', 'HEAD', '--', '.', ':!docs', ':!artifacts'],
                   cwd=ROOT, check=True)
    return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()


def run(approval):
    assert not any(path.exists() or path.is_symlink() for path in [*reuse.OWNED, *b.EXTRA, b.GENERATED])
    assert shutil.disk_usage(ROOT).free >= approval['requiredFreeBytes']
    subprocess.run(BUDGET + ['--approval-file', str(b.APPROVAL), '--planned-files', str(LIMITS['files']),
                            '--planned-bytes', str(LIMITS['logicalBytes']), '--planned-source-files',
                            str(LIMITS['sourceFiles'])], check=True)
    b.STATE.update(sourceCommit=verify_inputs(), expectReauthorization=True, normalBefore=reuse.normal_state())
    build_info = json.loads((ROOT / 'artifacts/build-info.json').read_text())
    archive = ROOT / 'artifacts' / build_info['file']
    executable = Path('Contents/MacOS/travel-gallery')
    assert sha(archive) == build_info['sha256'] and sha(b.NORMAL / executable) == build_info['executableSha256']
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(b.NORMAL)], check=True)
    b.STATE['acceptedReleaseBefore'] = build_info['sha256']
    b.OWNS_GENERATED = True
    b.TEMP = tempfile.TemporaryDirectory(prefix='travel-gallery-source-reauthorization-')
    b.WORK = Path(b.TEMP.name).resolve()
    b.STATE['temporaryRoot'] = str(b.WORK)
    b.save()
    cases = b.WORK / 'cases'
    cases.mkdir()
    env = b.prepare_home()
    env.update(TMPDIR=str(cases), CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0')
    b.command('fullRustRegressions', ['node', 'scripts/native.mjs', 'cargo', 'test', '--locked',
              '--manifest-path', 'src-tauri/Cargo.toml', '--', '--test-threads=1'], env)
    assert not any(cases.iterdir()), 'Rust fixtures must be removed before GUI acceptance'
    shutil.rmtree(b.WORK / 'target')
    candidate = b.build(False, env)
    b.STATE['normalCandidate'] = str(candidate)
    b.STATE['candidateExecutableSha256'] = sha(candidate / executable)
    fixture = b.WORK / 'fixture'
    source = fixture / 'source'
    source.mkdir(parents=True)
    reuse.write_sources(source)
    b.STATE['sourceBefore'] = reuse.source_state()
    marker = dict(identifier=b.IDENTIFIER, count=reuse.SOURCE_FILES, fixtureRoot=str(source),
                  scope='Candidate original read and picker restart')
    (fixture / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    stage = b.WORK / 'unsigned' / 'Travel Gallery Offline Bench.app'
    stage.parent.mkdir()
    b.command('copyCandidateForQA', ['ditto', str(candidate), str(stage)], env)
    assert sha(stage / executable) == b.STATE['candidateExecutableSha256']
    plist = stage / 'Contents/Info.plist'
    info = plistlib.loads(plist.read_bytes())
    info.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName=stage.stem, CFBundleDisplayName=stage.stem)
    plist.write_bytes(plistlib.dumps(info))
    b.command('sealCandidateQA', [sys.executable, 'scripts/prepare-offline-sandbox.py', '--source',
              str(stage), '--fixture', str(fixture), '--output', str(b.WORK / 'qa'), *reuse.SANDBOX_SEAL_ARGS], env)
    shutil.rmtree(stage.parent)
    b.APP_PATH = b.WORK / 'qa' / stage.name
    b.OWNED_PATHS.extend(reuse.OWNED)
    control = b.WORK / 'control'
    control.mkdir()
    b.STATE.update(status='ready', application=str(b.APP_PATH), expectedDatabase=str(b.DATA / 'catalog.sqlite'),
                   controlDirectory=str(control))
    b.event('readyForNativeGui', application=str(b.APP_PATH), source=str(source),
            controlDirectory=str(control), **b.check())
    checkpoints = []
    until = time.monotonic() + 1_800
    while time.monotonic() < until:
        b.check()
        for path in sorted(control.glob('*.json')):
            request = json.loads(path.read_text())
            path.unlink()
            if request['op'] == 'snapshot':
                label = request['label']
                assert len(checkpoints) < len(CHECKPOINTS) and label == CHECKPOINTS[len(checkpoints)]
                picker.snapshot(label)
                checkpoints.append(label)
            elif request['op'] == 'finish':
                assert tuple(checkpoints) == CHECKPOINTS and request['guiVerified'] is True
                processes = subprocess.check_output(['ps', '-axww', '-o', 'comm='], text=True).splitlines()
                assert str((b.APP_PATH / executable).resolve()) not in {path.strip() for path in processes}, (
                    'Quit the QA App before finishing'
                )
                assert reuse.source_state() == b.STATE['sourceBefore']
                assert verify_inputs() == b.STATE['sourceCommit']
                release = json.loads((ROOT / 'artifacts/build-info.json').read_text())
                assert release['executableSha256'] == sha(b.NORMAL / executable) == sha(candidate / executable)
                assert release['sha256'] == sha(ROOT / 'artifacts' / release['file'])
                subprocess.run(['codesign', '--verify', '--deep', '--strict', str(b.NORMAL)], check=True)
                b.STATE.update(status='finished', release=release, qaQuitNormally=True)
                return
            else:
                raise ValueError('Unknown QA control operation')
        time.sleep(.5)
    raise TimeoutError('No finish request before the QA window closed')


def main(approval):
    base_save, base_check = b.save, b.check

    def save():
        if b.STATE.get('temporaryDirectoryRemoved') and b.WORK:
            for key in ('temporaryRoot', 'application', 'expectedDatabase', 'controlDirectory',
                        'normalCandidate', 'originalPid', 'scanCheckpoints'):
                b.STATE.pop(key, None)
            for event in b.STATE['events']:
                event.pop('actualPid', None)
            b.STATE.update(json.loads(json.dumps(b.STATE).replace(str(b.WORK), '<cleaned run>')))
        base_save()

    def check():
        stats = base_check()
        if b.WORK:
            media = {'.png', '.jpg', '.jpeg', '.gif', '.webp', '.tiff', '.tif', '.bmp', '.mov', '.mp4', '.heic', '.raw'}
            count = sum(path.is_file() and path.suffix.lower() in media for path in (b.WORK / 'cases').rglob('*'))
            assert count <= LIMITS['sourceFiles']
            b.STATE['peakObservedRustMediaFiles'] = max(count, b.STATE.get('peakObservedRustMediaFiles', 0))
        return stats

    def checked_run():
        try:
            run(approval)
        finally:
            if 'normalBefore' in b.STATE:
                b.STATE['normalAfter'] = reuse.normal_state()
                b.STATE['normalPreserved'] = b.STATE['normalBefore'] == b.STATE['normalAfter']
                assert b.STATE['normalPreserved']

    b.save, b.check, b.main = save, check, checked_run
    result = b.run()
    postflight = subprocess.run(BUDGET, check=True, capture_output=True, text=True)
    b.STATE['postCleanupBudget'] = json.loads(postflight.stdout)
    b.save()
    assert not b.STATE['postCleanupBudget']['totalPlannedFiles']
    return result


if __name__ == '__main__':
    if not b.APPROVAL.is_file() or b.APPROVAL.is_symlink():
        raise SystemExit('Fresh batch approval is required')
    with b.APPROVAL.open('r+') as handle:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        record = json.load(handle)
        if (record.get('batch') != 'source-reauthorization' or record.get('status') != 'active'
                or record.get('userAnswer') != '允许这一轮，结束即清理' or record.get('claimedAt')
                or record.get('limits') != LIMITS or record.get('requiredFreeBytes') != LIMITS['logicalBytes']):
            raise SystemExit('This candidate needs fresh, unclaimed approval for its batch limits')
        record['claimedAt'] = datetime.now(timezone.utc).isoformat()
        handle.seek(0)
        json.dump(record, handle, ensure_ascii=False, indent=2)
        handle.truncate()
        handle.flush()
        raise SystemExit(main(record))
