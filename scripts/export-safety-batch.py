"""A separately approved export regression/build/GUI batch; reuse lifecycle cleanup."""
from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import sys
from datetime import datetime, timezone

sys.dont_write_bytecode=True
root=Path(__file__).resolve().parent.parent
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--native-only',action='store_true')
parser.add_argument('--approval-file',type=Path)
parser.add_argument('--receipt',type=Path)
args=parser.parse_args();native_only=args.native_only
approval=args.approval_file.resolve() if args.approval_file else root/('docs/export-safety-native-budget-20261031.json' if native_only else 'docs/export-safety-budget-20261031.json')
assert approval.parent==root/'docs'
if not approval.is_file():raise SystemExit('No approval for this export batch; no files generated.')
record=json.loads(approval.read_text())
if record.get('status')!='active' or record.get('userAnswer')!='允许这一轮，结束即清理':
    raise SystemExit('Export approval is not active; no files generated.')
if record.get('claimedAt'):
    raise SystemExit('This one-shot export approval was already claimed; no files generated.')
if native_only:
    verified=json.loads((root/'docs/export-safety-core-results.json').read_text())
    assert verified['passedTests']==60 and verified['windowsTypeCheckPassed']
    for name,digest in verified['sourceInputs'].items():
        assert hashlib.sha256((root/name).read_bytes()).hexdigest()==digest,name
control=json.loads((root/'docs/native-batch-control-check.json').read_text())
assert control['exitCode']==0 and control['approvalClosed'] and control['allTemporaryDataRemoved']
assert hashlib.sha256((root/'scripts/exit-confirmation-batch.py').read_bytes()).hexdigest()==control['controllerSourceSha256']
record['claimedAt']=datetime.now(timezone.utc).isoformat()
approval.write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n')
spec=importlib.util.spec_from_file_location('native_batch',Path(__file__).with_name('exit-confirmation-batch.py'))
batch=importlib.util.module_from_spec(spec);spec.loader.exec_module(batch)
batch.APPROVAL=approval
batch.RECEIPT=args.receipt.resolve() if args.receipt else root/('docs/export-safety-native-batch-latest.json' if native_only else 'docs/export-safety-batch-latest.json')
assert batch.RECEIPT.parent==root/'docs'
batch.WINDOW_TITLE='旅图 · 导出目录替换验收'
batch.TEMP_PREFIX='travel-gallery-export-batch-'
batch.FIXTURE_SCOPE='Five tiny owned PNGs; export publication, conflicts and target replacement only'
batch.PLANNED_SOURCE_FILES=5 if native_only else 500
batch.PLANNED_FILES=record.get('plannedFiles',18000)

original_check=batch.check
def check():
    stats=original_check()
    if batch.WORK:
        extensions={'.png','.jpg','.jpeg','.gif','.webp','.tiff','.bmp','.mov','.mp4','.heic','.raw'}
        media=sum(1 for p in (batch.WORK/'cases').rglob('*') if p.is_file() and p.suffix.lower() in extensions)
        native=list((batch.WORK/'fixture/source').glob('*'))
        batch.STATE['peakObservedCaseMediaFiles']=max(batch.STATE.get('peakObservedCaseMediaFiles',0),media)
        if media>500 or len(native)>5 or sum(p.stat().st_size for p in native)>10*1024**2:
            raise RuntimeError('Core/GUI source fixture limits exceeded')
    return stats
batch.check=check

def regressions(env):
    base=['node','scripts/native.mjs','cargo','test','--locked','--manifest-path','src-tauri/Cargo.toml']
    batch.command('coreUnitRegressions',base+['--lib','--','--test-threads=1'],env)
    batch.command('publicationRegressions',base+['--test','publish_boundaries','--','--test-threads=1'],env)
    batch.command('catalogExportRegressions',base+['--test','catalog_flow','export','--','--test-threads=1'],env)
    batch.command('windowsApiTypeCheck',[sys.executable,'scripts/check-windows-apis.py','--allow-network'],env)
    assert not list((batch.WORK/'cases').iterdir()),'A core fixture leaked before GUI dataset creation'

if not native_only:batch.regressions=regressions
raise SystemExit(batch.run())
