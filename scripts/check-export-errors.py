"""Run the real std-only export error policy tests and always remove the binary."""
from pathlib import Path
import datetime
import hashlib
import json
import subprocess
import sys
import tempfile

root=Path(__file__).resolve().parent.parent
source=root/'src-tauri/src/export_errors.rs'
subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py'),
    '--planned-files','20','--planned-bytes',str(32*1024**2)],check=True)
with tempfile.TemporaryDirectory(prefix='travel-gallery-export-errors-') as temporary:
    folder=Path(temporary);binary=folder/'error-policy-tests'
    compiled=subprocess.run([str(Path.home()/'.cargo/bin/rustc'),'--edition=2021','--test',
        '-C','debuginfo=0',str(source),'-o',str(binary)],capture_output=True,text=True)
    assert compiled.returncode==0,compiled.stderr[-4000:]
    result=subprocess.run([str(binary)],capture_output=True,text=True)
    size=sum(p.stat().st_size for p in folder.rglob('*') if p.is_file());assert size<=32*1024**2
    subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py'),'--extra-root',temporary],check=True)
report={'atUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'scope':'Actual std-only export_errors.rs policy tests; not full Catalog typing, filesystem or GUI acceptance.',
    'sourceSha256':hashlib.sha256(source.read_bytes()).hexdigest(),'testExit':result.returncode,
    'output':(result.stdout+result.stderr)[-8000:],'observedLogicalBytes':size,
    'temporaryDirectoryRemoved':not folder.exists()}
(root/'docs/export-error-policy-unit.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py')],check=True)
print(json.dumps(report,ensure_ascii=False));raise SystemExit(result.returncode)
