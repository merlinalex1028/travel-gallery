"""Build an isolated theme preview; remove its files and catalog after it exits."""
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
b.APPROVAL = ROOT / 'docs/native-theme-preview-budget.json'
b.RECEIPT = ROOT / 'docs/native-theme-preview-latest.json'
LIMITS = dict(files=25_000, logicalBytes=4 * 1024**3, sourceFiles=5)
b.EXTRA = list(cleanup.qa_namespace_paths(Path.home(), b.IDENTIFIER))
b.EXTRA += [
    b.LIBRARY.parent / 'tmp',
    b.LIBRARY / 'Application Support/CrashReporter',
]


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def formal_snapshot():
    paths = [
        b.NORMAL, ROOT / 'artifacts',
        Path.home() / 'Library/Application Support/com.travelgallery.desktop',
        Path.home() / 'Library/Caches/com.travelgallery.desktop',
        Path.home() / 'Library/Preferences/com.travelgallery.desktop.plist',
    ]
    result = {}
    for path in paths:
        entries = [path] if path.is_file() else sorted(path.rglob('*'))
        result[str(path)] = {
            'exists': path.exists(),
            'files': {str(p.relative_to(path) if path.is_dir() else p.name): sha(p)
                      for p in entries if p.is_file() and not p.is_symlink()},
        }
    return result


def processes():
    executable = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve())
    rows = subprocess.check_output(['ps', '-axo', 'pid=,comm='], text=True).splitlines()
    return [int(parts[0]) for row in rows
            if len(parts := row.strip().split(None, 1)) == 2 and parts[1] == executable]


def main():
    guard.require_unlocked_console()
    assert not list(cleanup.qa_data_targets())
    assert not b.GENERATED.exists()
    assert shutil.disk_usage(ROOT).free >= LIMITS['logicalBytes']
    b.STATE.update(sourceCommit=subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        formalBefore=formal_snapshot(), scope='User-requested packaged theme preview; no formal installation')
    b.TEMP = tempfile.TemporaryDirectory(prefix='travel-gallery-theme-native-')
    b.WORK = Path(b.TEMP.name).resolve()
    b.STATE['temporaryRoot'] = str(b.WORK)
    approval = json.loads(b.APPROVAL.read_text())
    approval['uniqueRunDirectory'] = str(b.WORK)
    b.APPROVAL.write_text(json.dumps(approval, ensure_ascii=False, indent=2) + '\n')
    subprocess.run([sys.executable, '-B', 'scripts/check-test-artifact-budget.py',
                    '--approval-file', str(b.APPROVAL), '--extra-root', str(b.WORK),
                    '--planned-files', str(LIMITS['files']),
                    '--planned-bytes', str(LIMITS['logicalBytes']),
                    '--planned-source-files', '5'], cwd=ROOT, check=True)
    b.OWNS_GENERATED = True
    env = b.prepare_home()
    env['CARGO_BUILD_JOBS'] = '4'
    front = b.frontend('production', env)
    # Sidecars are copied from the verified accepted App, with no worker changes.
    config = dict(
        identifier=b.IDENTIFIER,
        productName='Travel Gallery Offline Bench',
        build=dict(frontendDist=str(front)),
        bundle=dict(externalBin=[]),
        app=dict(windows=[dict(label='main', title='旅图 · 新样式预览', width=1440,
                               height=920, minWidth=860, minHeight=600)]),
    )
    b.command('compileThemePreview', [
        'node', 'scripts/native.mjs', 'cargo', 'build', '--release', '--locked',
        '--manifest-path', 'src-tauri/Cargo.toml', '--features',
        'desktop,custom-protocol', '--bin', 'travel-gallery',
    ], {**env, 'TAURI_CONFIG': json.dumps(config, ensure_ascii=False)})
    guard.require_unlocked_console()
    fixture = b.WORK / 'fixture'
    source = fixture / 'source'
    source.mkdir(parents=True)
    for index in range(5):
        output = b.WORK / f'raster-{index}'
        b.command(f'previewLandscape{index}', [
            'node', 'scripts/native.mjs', 'tauri', 'icon',
            f'public/demo/landscape-{index}.svg', '--png', '512', '--fit', 'cover',
            '--output', str(output),
        ], env)
        [image] = list(output.glob('*.png'))
        image.rename(source / f'journey-{index + 1:02}.png')
        shutil.rmtree(output)
    marker = dict(identifier=b.IDENTIFIER, count=5, fixtureRoot=str(source),
                  scope='Five project-owned demo illustrations for user theme preview')
    (fixture / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    stage = b.WORK / 'unsigned/Travel Gallery Offline Bench.app'
    stage.parent.mkdir()
    b.command('copyAcceptedBundleResources', ['ditto', str(b.NORMAL), str(stage)], env)
    shutil.copy2(b.WORK / 'target/release/travel-gallery', stage / 'Contents/MacOS/travel-gallery')
    info_path = stage / 'Contents/Info.plist'
    info = plistlib.loads(info_path.read_bytes())
    info.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName=stage.stem,
                CFBundleDisplayName=stage.stem)
    info_path.write_bytes(plistlib.dumps(info))
    icon = str(info.get('CFBundleIconFile', 'icon.icns'))
    assert Path(icon).name == icon
    if not Path(icon).suffix:
        icon += '.icns'
    shutil.copy2(ROOT / 'src-tauri/icons/icon.icns', stage / 'Contents/Resources' / icon)
    b.command('sealThemePreview', [
        sys.executable, '-B', 'scripts/prepare-offline-sandbox.py',
        '--source', str(stage), '--fixture', str(fixture),
        '--output', str(b.WORK / 'qa'), '--picker-only', '--webkit-mach-lookup',
    ], env)
    b.APP_PATH = b.WORK / 'qa/Travel Gallery Offline Bench.app'
    b.OWNED_PATHS.extend((b.DATA, b.CACHE))
    for name in ('unsigned', 'target', 'cargo-home', 'frontend-production'):
        shutil.rmtree(b.WORK / name)
    if b.GENERATED.exists():
        shutil.rmtree(b.GENERATED)
    for name in ('.vite', '.vite-temp'):
        path = ROOT / 'node_modules' / name
        if path.exists():
            cleanup.remove_target(path)
    b.STATE.update(status='ready', application=str(b.APP_PATH), source=str(source),
                   executableSha256=sha(b.APP_PATH / 'Contents/MacOS/travel-gallery'),
                   buildCachesRemoved=True, previewLifetimeSeconds=3600)
    b.event('readyForUserPreview', application=str(b.APP_PATH), source=str(source), **b.check())
    launched = False
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        b.check()
        active = processes()
        if active and not launched:
            launched = True
            deadline = time.monotonic() + 3600
            b.STATE.update(status='open-for-user', actualPid=active[0])
            b.event('previewProcessObserved', pid=active[0])
        if launched and not active:
            b.STATE.update(status='finished', userPreviewExited=True)
            return
        time.sleep(1)
    raise TimeoutError('Theme preview launch/viewing time limit reached; cleaning owned resources')


if __name__ == '__main__':
    guard.require_unlocked_console()
    assert not b.APPROVAL.exists() or json.loads(b.APPROVAL.read_text())['status'] == 'closed'
    authority = ROOT / 'AGENTS.md'
    b.APPROVAL.write_text(json.dumps(dict(
        status='active', batch='native-theme-preview', limits=LIMITS,
        requiredFreeBytes=LIMITS['logicalBytes'],
        authorizationBasis='AGENTS.md:standing-resource-2026-11-23',
        authorizationFileSha256=sha(authority), cleanupPolicy='end-of-batch',
        deletionTime='Remove build caches before launch; remove preview App, fixtures and QA data on user exit, interruption or one-hour timeout',
        registeredAt=datetime.now(timezone.utc).isoformat(),
    ), ensure_ascii=False, indent=2) + '\n')
    b.main = main
    result = b.run()
    if 'formalBefore' in b.STATE:
        b.STATE['formalAfter'] = formal_snapshot()
        b.STATE['formalPreserved'] = b.STATE['formalBefore'] == b.STATE['formalAfter']
        assert b.STATE['formalPreserved']
    b.STATE['postCleanupBudget'] = json.loads(subprocess.check_output(
        [sys.executable, '-B', 'scripts/check-test-artifact-budget.py'], cwd=ROOT, text=True))
    b.save()
    # A preview can outlive the opening request; checkpoint only its own receipts.
    subprocess.run(['git', 'add', '--', str(b.APPROVAL), str(b.RECEIPT)], cwd=ROOT, check=True)
    subprocess.run(['git', 'commit', '--only', '-m', 'docs: 关闭并清理原生主题预览批次',
                    '--', str(b.APPROVAL), str(b.RECEIPT)], cwd=ROOT, check=True)
    raise SystemExit(result)
