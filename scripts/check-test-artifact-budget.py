"""Read-only preflight: count logical bytes even for shared APFS clones."""
from pathlib import Path
import argparse
import importlib.util
import json
import sys

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location('qa_cleanup', Path(__file__).with_name('cleanup-test-artifacts.py'))
cleanup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cleanup)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--planned-files', type=int, default=0)
parser.add_argument('--planned-source-files', type=int, default=0)
parser.add_argument('--planned-bytes', type=int, default=0)
parser.add_argument('--extra-root', action='append', type=Path, default=[])
parser.add_argument('--approval-file', type=Path)
args = parser.parse_args()
assert min(args.planned_files,args.planned_source_files,args.planned_bytes) >= 0
info = cleanup.ARTIFACTS / 'build-info.json'
kept = {'build-info.json','SHA256SUMS.txt'}
if info.exists():
    kept.add(json.loads(info.read_text())['file'])
roots = [p for p in cleanup.ARTIFACTS.iterdir() if p.name not in kept] if cleanup.ARTIFACTS.exists() else []
roots += [cleanup.ROOT/name for name in ('dist-offline-canary','dist-performance','test-results') if (cleanup.ROOT/name).exists()]
roots += list(cleanup.build_cache_targets())
generated=cleanup.ROOT/'src-tauri/gen'
if generated.exists() or generated.is_symlink():roots.append(generated)
roots += list(cleanup.qa_data_targets())
roots += [p for p in args.extra_root if p.exists() or p.is_symlink()]
stats=[cleanup.measure(p) for p in roots]
files=sum(s['files'] for s in stats)+args.planned_files
logical=sum(s['logicalBytes'] for s in stats)+args.planned_bytes
limits=dict(files=2000,logicalBytes=2*1024**3,sourceFiles=500)
if args.approval_file:
    approval_path=args.approval_file.resolve(strict=True)
    assert approval_path.parent==cleanup.ROOT/'docs'
    approval=json.loads(approval_path.read_text())
    assert approval['status']=='active' and approval['userAnswer']=='允许这一轮，结束即清理'
    limits=approval['limits']
passed=files<=limits['files'] and logical<=limits['logicalBytes'] and args.planned_source_files<=limits['sourceFiles']
print(json.dumps(dict(passed=passed,totalPlannedFiles=files,totalPlannedLogicalBytes=logical,
    plannedSourceFiles=args.planned_source_files,limits=limits,
    note='A larger workload needs an explicit user-approved size/count/cleanup plan; APFS sharing does not lower this logical budget.'),ensure_ascii=False))
raise SystemExit(0 if passed else 1)
