"""One approved production build: query-scroll reset and full exact-group estimate."""
from pathlib import Path
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
import plistlib
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
APPROVAL = ROOT/'docs/query-scroll-native-budget.json'
if not APPROVAL.is_file():
    raise SystemExit('No approval; no temporary files generated.')
approval = json.loads(APPROVAL.read_text())
if approval.get('status') != 'active' or approval.get('userAnswer') != '允许这一轮，结束即清理' or approval.get('claimedAt'):
    raise SystemExit('This batch needs fresh, unclaimed approval; no temporary files generated.')
frozen = json.loads((ROOT/'docs/query-scroll-build-inputs.json').read_text())
for name, digest in {**frozen['sourceInputs'], **frozen['harnessInputs']}.items():
    assert hashlib.sha256((ROOT/name).read_bytes()).hexdigest() == digest, name
checks = json.loads((ROOT/'docs/query-scroll-checks.json').read_text())
assert checks['fullFrontend']['exitCode'] == checks['strictTypeScript']['exitCode'] == checks['productionFrontend']['exitCode'] == 0
estimate = json.loads((ROOT/'docs/duplicate-redundancy-checks.json').read_text())
assert estimate['fullFrontend']['exitCode'] == estimate['strictTypeScript']['exitCode'] == estimate['productionFrontend']['exitCode'] == 0
control = json.loads((ROOT/'docs/native-batch-control-check.json').read_text())
assert control['exitCode'] == 0 and control['approvalClosed'] and control['allTemporaryDataRemoved']
assert hashlib.sha256((ROOT/'scripts/exit-confirmation-batch.py').read_bytes()).hexdigest() == control['controllerSourceSha256']
spec = importlib.util.spec_from_file_location('sorting', ROOT/'scripts/native-sorting-batch.py')
sorting = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sorting)
reuse, b = sorting.reuse, sorting.reuse.b
from native_fixture_budget import source_inventory

b.APPROVAL = APPROVAL
b.RECEIPT = ROOT/'docs/query-scroll-native-latest.json'
b.DATA = b.LIBRARY/'Application Support/com.travelgallery.desktop'
b.CACHE = b.LIBRARY/'Caches/com.travelgallery.desktop'
b.PLANNED_FILES = 40000
reuse.RUN_PREFIX = 'travel-gallery-query-scroll-build-'
original_check, original_build = b.check, b.build


def check():
    stats = original_check()
    if b.WORK:
        source = source_inventory(b.WORK/'fixture')
        count, size = len(source), sum(p.stat().st_size for p in source)
        if count > 125 or size > 1024**2:
            raise RuntimeError(f'Native source budget exceeded: {count} files / {size} bytes')
        b.STATE['peakNativeSourceFiles'] = max(b.STATE.get('peakNativeSourceFiles', 0), count)
    return stats


def build(qa, env):
    assert not qa, 'This batch uses one production build, then an isolated signed copy'
    if not b.STATE.get('normalCandidate'):
        b.STATE['normalCandidate'] = str(original_build(False, env))
        b.save()
    return Path(b.STATE['normalCandidate'])


def write_sources(source):
    for index in range(125):
        capture, offset = ('2024:03:24 08:00:00', None) if index < 121 else (
            ('2024:03:23 23:00:00', '+08:00') if index == 121 else
            ('2024:03:24 10:00:00', '+08:00') if index == 122 else (None, None))
        path = source/f'sort-{index:03}.tiff'
        # 121 identical files also exercise estimates across member pages.
        path.write_bytes(sorting.tiny_tiff(0 if index < 121 else index, capture, offset))
        stamp = datetime(2024, 3, 24 if index != 124 else 22, 8, tzinfo=timezone.utc).timestamp()
        os.utime(path, ns=(int(stamp*10**9), int(stamp*10**9)))


def main():
    assert not any(p.exists() or p.is_symlink() for p in [*reuse.OWNED, *b.EXTRA]), 'QA namespace must be empty'
    assert not b.GENERATED.exists() and not b.GENERATED.is_symlink()
    assert shutil.disk_usage(ROOT).free >= approval['requiredFreeBytes'], 'Insufficient free space for this batch'
    b.OWNS_GENERATED = True
    b.STATE['normalBefore'] = reuse.normal_state()
    info = json.loads((ROOT/'artifacts/build-info.json').read_text())
    assert hashlib.sha256((b.NORMAL/'Contents/MacOS/travel-gallery').read_bytes()).hexdigest() == info['executableSha256']
    b.STATE['acceptedReleaseBefore'] = info
    subprocess.run([sys.executable, str(ROOT/'scripts/check-test-artifact-budget.py'),
        '--approval-file', str(APPROVAL), '--planned-files', '40000',
        '--planned-bytes', str(5*1024**3), '--planned-source-files', '125'], check=True)
    b.TEMP = reuse.RunDirectory()
    b.WORK = Path(b.TEMP.name)
    b.STATE['temporaryRoot'] = str(b.WORK)
    b.save()
    (b.WORK/'cases').mkdir()
    env = b.prepare_home()
    env['TMPDIR'] = str(b.WORK/'cases')
    fixture = b.WORK/'fixture'
    source = fixture/'source'
    source.mkdir(parents=True)
    write_sources(source)
    b.STATE['sourceBefore'] = reuse.source_state()
    marker = dict(identifier=b.IDENTIFIER, count=125, fixtureRoot=str(source),
        scope='Query/sort reset plus121-member exact-group estimate; 125 tiny TIFFs; one production build')
    (fixture/'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    candidate = build(False, env)
    stage = b.WORK/'qa-unsigned/Travel Gallery Offline Bench.app'
    stage.parent.mkdir()
    b.command('copyProductionForQA', ['ditto', str(candidate), str(stage)], env)
    digest = hashlib.sha256((stage/'Contents/MacOS/travel-gallery').read_bytes()).hexdigest()
    assert digest == hashlib.sha256((candidate/'Contents/MacOS/travel-gallery').read_bytes()).hexdigest()
    b.STATE['qaProductionExecutableBeforeSigning'] = digest
    p = stage/'Contents/Info.plist'
    info = plistlib.loads(p.read_bytes())
    info.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName='Travel Gallery Offline Bench', CFBundleDisplayName='Travel Gallery Offline Bench')
    p.write_bytes(plistlib.dumps(info))
    b.command('sealProductionQA', [sys.executable, 'scripts/prepare-offline-sandbox.py',
        '--source', str(stage), '--fixture', str(fixture), '--output', str(b.WORK/'qa'), '--webkit-mach-lookup'], env)
    shutil.rmtree(stage.parent)
    b.APP_PATH = b.WORK/'qa/Travel Gallery Offline Bench.app'
    for path in [b.DATA, b.CACHE]:
        path.mkdir(parents=True)
        b.OWNED_PATHS.append(path)
        (path/'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    b.STATE.update(status='ready', application=str(b.APP_PATH), expectedDatabase=str(b.DATA/'catalog.sqlite'))
    b.event('readyForNativeGui', application=str(b.APP_PATH), source=str(source), **check())
    b.control_loop(env)


b.check, b.build, b.main = check, build, main
approval['claimedAt'] = datetime.now(timezone.utc).isoformat()
APPROVAL.write_text(json.dumps(approval, ensure_ascii=False, indent=2)+'\n')
raise SystemExit(b.run())
