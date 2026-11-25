"""Build the approved logo, verify an isolated native copy, publish, then clean."""
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


b = load('exit-confirmation-batch')
cleanup = load('cleanup-test-artifacts')
guard = load('native-gui-preflight')
b.APPROVAL = ROOT / 'docs/native-brand-release-budget.json'
b.RECEIPT = ROOT / 'docs/native-brand-release-latest.json'
LIMITS = dict(files=25_000, logicalBytes=4 * 1024**3, sourceFiles=1)
b.EXTRA = list(cleanup.qa_namespace_paths(Path.home(), b.IDENTIFIER))
b.EXTRA += [b.LIBRARY.parent / 'tmp', b.LIBRARY / 'Application Support/CrashReporter']
SYSTEM_TEMP = Path(tempfile.gettempdir()).resolve()
for name in [b.IDENTIFIER, *[
        f'com.apple.WebKit.{kind}+{b.IDENTIFIER}' for kind in ('WebContent', 'Networking', 'GPU')]]:
    b.EXTRA.append(SYSTEM_TEMP.parent / 'C' / name)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def formal_data():
    paths = [Path.home() / 'Library' / folder / name for folder, name in (
        ('Application Support', 'com.travelgallery.desktop'),
        ('Caches', 'com.travelgallery.desktop'),
        ('Preferences', 'com.travelgallery.desktop.plist'),
        ('Containers', 'com.travelgallery.desktop'),
    )]
    return {str(path): dict(exists=path.exists(), files={
        str(p.relative_to(path) if path.is_dir() else p.name): sha(p)
        for p in ([path] if path.is_file() else path.rglob('*'))
        if p.is_file() and not p.is_symlink()
    }) for path in paths}


def active():
    executable = str(b.APP_PATH / 'Contents/MacOS/travel-gallery')
    return [int(parts[0]) for row in subprocess.check_output(
        ['ps', '-axo', 'pid=,comm='], text=True).splitlines()
        if len(parts := row.strip().split(None, 1)) == 2 and parts[1] == executable]


def build_candidate(env):
    front = b.frontend('production', env)
    # Helpers are unchanged. The accepted formal bundle supplies their signed bytes;
    # compiling the main executable must not require deleted intermediate sidecars.
    config = dict(build=dict(frontendDist=str(front)), bundle=dict(externalBin=[]))
    b.command('compileBrandCandidate', [
        'node', 'scripts/native.mjs', 'cargo', 'build', '--release', '--locked',
        '--manifest-path', 'src-tauri/Cargo.toml', '--features',
        'desktop,custom-protocol', '--bin', 'travel-gallery',
    ], {**env, 'TAURI_CONFIG': json.dumps(config)})
    candidate = b.WORK / 'normal-candidate/Travel Gallery.app'
    candidate.parent.mkdir()
    b.command('copyAcceptedResources', ['ditto', str(b.NORMAL), str(candidate)], env)
    shutil.copy2(b.WORK / 'target/release/travel-gallery', candidate / 'Contents/MacOS/travel-gallery')
    return candidate


def publish(candidate, env):
    previous = json.loads((ROOT / 'artifacts/build-info.json').read_text())
    rollback = b.WORK / 'rollback'
    rollback.mkdir()
    files = [previous['file'], 'build-info.json', 'SHA256SUMS.txt']
    for name in files:
        shutil.copy2(ROOT / 'artifacts' / name, rollback / name)
    old_app = rollback / b.NORMAL.name
    installed = False
    try:
        b.command('packageApprovedBrand', ['node', 'scripts/package-macos.mjs'],
                  {**env, 'TRAVEL_GALLERY_PACKAGE_APP': str(candidate)})
        release = json.loads((ROOT / 'artifacts/build-info.json').read_text())
        assert sha(candidate / 'Contents/MacOS/travel-gallery') == release['executableSha256']
        b.NORMAL.rename(old_app)
        candidate.rename(b.NORMAL)
        assert sha(b.NORMAL / 'Contents/Resources/icon.icns') == sha(ROOT / 'src-tauri/icons/icon.icns')
        assert sha(ROOT / 'artifacts' / release['file']) == release['sha256']
        b.STATE['release'] = release
        installed = True
    finally:
        if not installed:
            if old_app.exists():
                if b.NORMAL.exists():
                    b.NORMAL.rename(candidate)
                old_app.rename(b.NORMAL)
            for name in files:
                (rollback / name).replace(ROOT / 'artifacts' / name)


def main():
    guard.require_unlocked_console()
    assert not list(cleanup.qa_data_targets())
    assert not b.LIBRARY.parent.exists(), 'A previous isolated Data directory must be cleaned first'
    assert not b.GENERATED.exists()
    assert shutil.disk_usage(ROOT).free >= LIMITS['logicalBytes']
    b.STATE.update(sourceCommit=subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        formalDataBefore=formal_data(), scope='Approved B logo; native icon and UI acceptance without importing photos')
    b.TEMP = tempfile.TemporaryDirectory(prefix='travel-gallery-brand-release-')
    b.WORK = Path(b.TEMP.name).resolve()
    b.STATE['temporaryRoot'] = str(b.WORK)
    approval = json.loads(b.APPROVAL.read_text())
    approval['uniqueRunDirectory'] = str(b.WORK)
    b.APPROVAL.write_text(json.dumps(approval, ensure_ascii=False, indent=2) + '\n')
    subprocess.run([sys.executable, '-B', 'scripts/check-test-artifact-budget.py',
                    '--approval-file', str(b.APPROVAL), '--extra-root', str(b.WORK),
                    '--planned-files', str(LIMITS['files']), '--planned-bytes',
                    str(LIMITS['logicalBytes']), '--planned-source-files', '1'], cwd=ROOT, check=True)
    b.OWNS_GENERATED = True
    env = b.prepare_home()
    env['CARGO_BUILD_JOBS'] = '4'
    candidate = build_candidate(env)
    b.STATE['normalCandidate'] = str(candidate)
    shutil.copy2(ROOT / 'src-tauri/icons/icon.icns', candidate / 'Contents/Resources/icon.icns')
    info_path = candidate / 'Contents/Info.plist'
    info = plistlib.loads(info_path.read_bytes())
    info['CFBundleIconFile'] = 'icon.icns'
    info_path.write_bytes(plistlib.dumps(info))
    b.command('sealBrandCandidate', ['codesign', '--force', '--sign', '-',
                                    '--timestamp=none', str(candidate)], env)
    b.command('verifyBrandCandidate', ['codesign', '--verify', '--deep', '--strict', str(candidate)], env)
    guard.require_unlocked_console()
    fixture = b.WORK / 'fixture'
    source = fixture / 'source'
    source.mkdir(parents=True)
    shutil.copy2(ROOT / 'src-tauri/icons/icon.png', source / 'brand.png')
    (fixture / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(dict(
        identifier=b.IDENTIFIER, count=1, fixtureRoot=str(source),
        scope='One project-owned icon for sandbox preparation; no library import')))
    unsigned = b.WORK / 'unsigned/Travel Gallery Offline Bench.app'
    unsigned.parent.mkdir()
    b.command('copyBrandForQA', ['ditto', str(candidate), str(unsigned)], env)
    plist = unsigned / 'Contents/Info.plist'
    info = plistlib.loads(plist.read_bytes())
    info.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName=unsigned.stem,
                CFBundleDisplayName=unsigned.stem)
    plist.write_bytes(plistlib.dumps(info))
    b.command('sealBrandQA', [sys.executable, '-B', 'scripts/prepare-offline-sandbox.py',
                             '--source', str(unsigned), '--fixture', str(fixture),
                             '--output', str(b.WORK / 'qa'), '--picker-only', '--webkit-mach-lookup'], env)
    b.APP_PATH = b.WORK / 'qa/Travel Gallery Offline Bench.app'
    b.OWNED_PATHS.append(b.LIBRARY.parent)
    for name in ('unsigned', 'target', 'cargo-home', 'frontend-production'):
        shutil.rmtree(b.WORK / name)
    if b.GENERATED.exists():
        shutil.rmtree(b.GENERATED)
    control = b.WORK / 'native-check.json'
    b.STATE.update(status='ready', application=str(b.APP_PATH), control=str(control),
                   iconSha256=sha(ROOT / 'src-tauri/icons/icon.icns'), buildCachesRemoved=True)
    b.event('readyForBrandAcceptance', application=str(b.APP_PATH), control=str(control), **b.check())
    deadline = time.monotonic() + 900
    while time.monotonic() < deadline:
        guard.require_unlocked_console()
        b.check()
        if control.is_file():
            result = json.loads(control.read_text())
            assert result['passed'] is True
            assert not active(), 'Exit the isolated application before accepting and publishing'
            b.STATE['nativeAcceptance'] = result
            break
        time.sleep(.5)
    else:
        raise TimeoutError('Native logo acceptance did not finish within 15 minutes')
    assert formal_data() == b.STATE['formalDataBefore']
    publish(candidate, env)
    b.STATE.update(status='finished', formalDataUnchanged=formal_data() == b.STATE['formalDataBefore'])
    assert b.STATE['formalDataUnchanged']


if __name__ == '__main__':
    guard.require_unlocked_console()
    assert not b.APPROVAL.exists() or json.loads(b.APPROVAL.read_text())['status'] == 'closed'
    if b.RECEIPT.exists():
        previous = json.loads(b.RECEIPT.read_text())
        if previous.get('status') == 'failed':
            b.STATE['previousAttempt'] = {key: previous.get(key) for key in (
                'startedAt', 'finishedAt', 'error', 'temporaryDirectoryRemoved')}
    b.APPROVAL.write_text(json.dumps(dict(
        status='active', batch='native-brand-release', limits=LIMITS,
        requiredFreeBytes=LIMITS['logicalBytes'],
        authorizationBasis='AGENTS.md:standing-resource-2026-11-23',
        authorizationFileSha256=sha(ROOT / 'AGENTS.md'), cleanupPolicy='end-of-batch',
        deletionTime='Remove compilation/download caches before UI acceptance; remove QA App, fixture, isolated data, rollback and run directory after acceptance or failure',
        registeredAt=datetime.now(timezone.utc).isoformat(),
    ), ensure_ascii=False, indent=2) + '\n')
    b.main = main
    result = b.run()
    for name in ('.vite', '.vite-temp'):
        path = ROOT / 'node_modules' / name
        if path.exists():
            cleanup.remove_target(path)
    b.STATE['postCleanupBudget'] = json.loads(subprocess.check_output(
        [sys.executable, '-B', 'scripts/check-test-artifact-budget.py'], cwd=ROOT, text=True))
    b.save()
    raise SystemExit(result)
