"""One explicitly approved snapshot-exclusion regression/build/GUI batch."""
from pathlib import Path
from datetime import datetime, timezone
import argparse
import hashlib
import importlib.util
import json
import plistlib
import shutil
import sys

sys.dont_write_bytecode = True
from native_fixture_budget import source_inventory

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--native-only', action='store_true')
native_only = parser.parse_args().native_only
root = Path(__file__).resolve().parent.parent
approval = root/('docs/snapshot-exclusions-native-budget.json' if native_only else 'docs/snapshot-exclusions-build-budget.json')
if not approval.is_file():
    raise SystemExit('No approval for this batch; no files generated.')
record = json.loads(approval.read_text())
if record.get('status') != 'active' or record.get('userAnswer') != '允许这一轮，结束即清理' or record.get('claimedAt'):
    raise SystemExit('This batch needs fresh, unclaimed approval; no files generated.')
control = json.loads((root/'docs/native-batch-control-check.json').read_text())
assert control['exitCode'] == 0 and control['approvalClosed'] and control['allTemporaryDataRemoved']
assert hashlib.sha256((root/'scripts/exit-confirmation-batch.py').read_bytes()).hexdigest() == control['controllerSourceSha256']
if native_only:
    verified = json.loads((root/'docs/snapshot-exclusions-checks.json').read_text())
    assert verified['fullRust']['exitCode'] == 0 and verified['fullRust']['passed'] == 127
    frozen = json.loads((root/'docs/snapshot-exclusions-build-inputs.json').read_text())
    for name, digest in frozen['sourceInputs'].items():
        assert hashlib.sha256((root/name).read_bytes()).hexdigest() == digest, name
spec = importlib.util.spec_from_file_location('reuse', Path(__file__).with_name('native-disk-full-batch.py'))
reuse = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reuse)
b = reuse.b
b.APPROVAL = approval
b.RECEIPT = root/('docs/snapshot-exclusions-native-latest.json' if native_only else 'docs/snapshot-exclusions-build-latest.json')
b.WINDOW_TITLE = '旅图 · 冻结选区排除项验收'
b.TEMP_PREFIX = 'travel-gallery-snapshot-build-'
b.FIXTURE_SCOPE = 'Five tiny owned PNGs; frozen snapshot exclusions, new-asset membership, review/undo and export plan scope'
b.PLANNED_SOURCE_FILES = 5 if native_only else 500
b.PLANNED_FILES = 40000
data_identifier = 'com.travelgallery.desktop' if native_only else b.IDENTIFIER
b.DATA = b.LIBRARY/'Application Support'/data_identifier
b.CACHE = b.LIBRARY/'Caches'/data_identifier


def regressions(env):
    if native_only:
        b.event('reuseVerifiedRustResults', passed=127, ignored=9, frozenInputs=len(frozen['sourceInputs']))
        return
    b.command('fullRustRegressions', ['node', 'scripts/native.mjs', 'cargo', 'test', '--locked',
        '--manifest-path', 'src-tauri/Cargo.toml', '--', '--test-threads=1'], env)
    assert not list((b.WORK/'cases').iterdir()), 'A core fixture leaked before GUI preparation'


original_check = b.check


def check():
    stats = original_check()
    if b.WORK:
        extensions = {'.png', '.jpg', '.jpeg', '.gif', '.webp', '.tiff', '.bmp', '.mov', '.mp4', '.heic', '.raw'}
        media = [p for p in (b.WORK/'cases').rglob('*') if p.is_file() and p.suffix.lower() in extensions]
        native = source_inventory(b.WORK/'fixture')
        if len(media) > 500:
            raise RuntimeError(f'Rust fixture input limit exceeded: {len(media)} media files')
        if len(native) > 5 or sum(p.stat().st_size for p in native) > 10*1024**2:
            raise RuntimeError(f'Native input limit exceeded: {len(native)} source files')
        b.STATE['peakObservedCaseMediaFiles'] = max(b.STATE.get('peakObservedCaseMediaFiles', 0), len(media))
    return stats


b.regressions = regressions
b.check = check
original_build = b.build


def build(qa, env):
    if not native_only:
        return original_build(qa, env)
    if not qa:
        return Path(b.STATE['normalCandidate'])
    candidate = original_build(False, env)
    b.STATE['normalCandidate'] = str(candidate)
    stage = b.WORK/'qa-unsigned'
    stage.mkdir()
    app = stage/'Travel Gallery Offline Bench.app'
    b.command('copyProductionForQA', ['ditto', str(candidate), str(app)], env)
    before = hashlib.sha256((app/'Contents/MacOS/travel-gallery').read_bytes()).hexdigest()
    assert before == hashlib.sha256((candidate/'Contents/MacOS/travel-gallery').read_bytes()).hexdigest()
    b.STATE['qaProductionExecutableBeforeSigning'] = before
    p = app/'Contents/Info.plist'
    info = plistlib.loads(p.read_bytes())
    info.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName='Travel Gallery Offline Bench', CFBundleDisplayName='Travel Gallery Offline Bench')
    p.write_bytes(plistlib.dumps(info))
    b.command('sealProductionQA', [sys.executable, 'scripts/prepare-offline-sandbox.py', '--source', str(app),
        '--fixture', str(b.WORK/'fixture'), '--output', str(b.WORK/'qa'), '--webkit-mach-lookup'], env)
    shutil.rmtree(stage)
    return b.WORK/'qa/Travel Gallery Offline Bench.app'


b.build = build
original_main = b.main
original_control_loop = b.control_loop


def control_loop(env):
    b.STATE['sourceBefore'] = reuse.source_state()
    b.save()
    original_control_loop(env)


def main():
    b.STATE['normalBefore'] = reuse.normal_state()
    info = json.loads((root/'artifacts/build-info.json').read_text())
    assert hashlib.sha256((b.NORMAL/'Contents/MacOS/travel-gallery').read_bytes()).hexdigest() == info['executableSha256']
    b.STATE['acceptedReleaseBefore'] = info
    try:
        original_main()
    finally:
        b.STATE['normalAfter'] = reuse.normal_state()
        b.STATE['normalPreserved'] = b.STATE['normalBefore'] == b.STATE['normalAfter']
        if b.WORK and (b.WORK/'fixture/source').exists() and 'sourceBefore' in b.STATE:
            b.STATE['sourceAfter'] = reuse.source_state()
            b.STATE['sourceUnchanged'] = b.STATE['sourceBefore'] == b.STATE['sourceAfter']
        b.save()


b.main = main
b.control_loop = control_loop
record['claimedAt'] = datetime.now(timezone.utc).isoformat()
approval.write_text(json.dumps(record, ensure_ascii=False, indent=2)+'\n')
raise SystemExit(b.run())
