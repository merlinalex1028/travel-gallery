"""Run dependency-free exit-state regression tests; always delete the test binary."""
from pathlib import Path
import argparse
import datetime
import hashlib
import json
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--expect-failure', action='store_true')
parser.add_argument('--report', type=Path, default=root/'docs/exit-guard-unit-latest.json')
args = parser.parse_args()
source = root/'src-tauri/src/exit_guard.rs'
report_path = args.report.absolute()
assert report_path.parent == root/'docs'
subprocess.run([sys.executable, str(root/'scripts/check-test-artifact-budget.py'),
    '--planned-files','20','--planned-bytes',str(32*1024**2)],check=True)
with tempfile.TemporaryDirectory(prefix='travel-gallery-exit-unit-') as temporary:
    output = Path(temporary)/'exit-guard-tests'
    compiled = subprocess.run([str(Path.home()/'.cargo/bin/rustc'),'--edition=2021','--test',
        '-C','debuginfo=0',str(source),'-o',str(output)],capture_output=True,text=True)
    if compiled.returncode:
        raise RuntimeError(compiled.stderr[-4000:])
    result = subprocess.run([str(output)],capture_output=True,text=True)
    footprint = sum(p.stat().st_size for p in Path(temporary).rglob('*') if p.is_file())
    assert footprint <= 32*1024**2
    subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py'),
        '--extra-root',temporary],check=True)
report = dict(atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
    scope='Standalone std-only exit-state tests. Does not compile Tauri or prove native menu/dialog behavior.',
    sourceSha256=hashlib.sha256(source.read_bytes()).hexdigest(),expectedFailure=args.expect_failure,
    compilerExit=compiled.returncode,testExit=result.returncode,testOutput=(result.stdout+result.stderr)[-8000:],
    peakObservedTestBinaryBytes=footprint,temporaryDirectoryRemoved=not Path(temporary).exists())
assert report['temporaryDirectoryRemoved']
report_path.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py')],check=True)
print(json.dumps(report,ensure_ascii=False))
raise SystemExit(0 if (result.returncode!=0)==args.expect_failure else 1)
