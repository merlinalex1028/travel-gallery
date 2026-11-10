"""Exercise actual control, child shutdown and cleanup with closed stdin; no builds."""
from pathlib import Path
import datetime
import hashlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time

root=Path(__file__).resolve().parent.parent
subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py'),
    '--planned-files','20','--planned-bytes',str(1024**2)],check=True)
with tempfile.TemporaryDirectory(prefix='travel-gallery-controller-check-') as temporary:
    folder=Path(temporary)
    code=r'''
import importlib.util,json,sys,tempfile,shutil,subprocess,signal
from pathlib import Path
sys.dont_write_bytecode=True
root,folder=map(Path,sys.argv[1:3]);mode=sys.argv[3]
spec=importlib.util.spec_from_file_location('batch',root/'scripts/exit-confirmation-batch.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
m.TEMP=tempfile.TemporaryDirectory(prefix='run-',dir=folder)
m.WORK=Path(m.TEMP.name);work=m.WORK
if mode=='finish':
    empty_registry=work/'empty-registry';empty_registry.mkdir()
    environment=m.prepare_home(empty_registry)
    assert Path(environment['CARGO_HOME']).is_relative_to(work)
    assert shutil.which('rustc',path=environment['PATH'])
    assert subprocess.run(['rustc','-vV'],env=environment,capture_output=True).returncode==0
m.RECEIPT=folder/'receipt.json';m.APPROVAL=folder/'approval.json'
m.APPROVAL.write_text(json.dumps({'status':'active','scope':'Controller-only unit fixture; no native compilation'}))
m.OWNED_PATHS=[folder/'owned-data',folder/'owned-cache']
m.EXTRA=[folder/'owned-extra'];m.GENERATED=folder/'owned-generated';m.OWNS_GENERATED=True
owned=[*m.OWNED_PATHS,*m.EXTRA,m.GENERATED]
for path in owned:
    path.mkdir();(path/'sentinel').write_text('Temporary cleanup fixture')
m.check=lambda:{'files':0,'logicalBytes':0}
calls=[]
m.build=lambda qa,env:calls.append('normal') or work/'candidate/Travel Gallery.app'
m.snapshot=lambda label:calls.append(label)
if mode=='finish':
    control=work/'control';control.mkdir()
    for number,request in enumerate(({'op':'normal'},{'op':'snapshot','label':'observed'},{'op':'finish'})):
        pending=control/f'{number:03}.tmp';pending.write_text(json.dumps(request));pending.rename(pending.with_suffix('.json'))
    m.main=lambda:m.control_loop({})
else:
    child_code="import os,time,signal,sys;from pathlib import Path;signal.signal(signal.SIGTERM,lambda s,f:(time.sleep(.15),sys.exit(0)));Path("+repr(str(folder/'child-ready'))+").write_text(str(os.getpid()));time.sleep(60)"
    m.main=lambda:m.command('ownedStub',[sys.executable,'-c',child_code])
handlers={s:signal.getsignal(s) for s in m.INTERRUPTION_SIGNALS}
exit_code=m.run()
assert handlers=={s:signal.getsignal(s) for s in m.INTERRUPTION_SIGNALS}
assert exit_code==(0 if mode=='finish' else 1),exit_code
if mode=='finish':assert calls==['normal','observed'],calls
else:assert m.STATE['terminationSignal']==mode,m.STATE
assert not work.exists()
assert not any(path.exists() for path in owned)
assert json.loads(m.APPROVAL.read_text())['status']=='closed'
assert json.loads(m.RECEIPT.read_text())['temporaryDirectoryRemoved']
print('CONTROLLER_AND_CLEANUP_PASSED:'+mode)
'''
    checks=[]
    for mode in ('finish','SIGTERM','SIGHUP','SIGINT'):
        case=folder/mode;case.mkdir()
        process=subprocess.Popen([sys.executable,'-c',code,str(root),str(case),mode],
            stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        child_pid=None
        try:
            if mode!='finish':
                deadline=time.monotonic()+10
                ready=case/'child-ready'
                while not ready.exists():
                    assert process.poll() is None,'Controller exited before child became ready'
                    assert time.monotonic()<deadline,'Child readiness timed out'
                    time.sleep(.02)
                child_pid=int(ready.read_text())
                process.send_signal(getattr(signal,mode))
                time.sleep(.03)
                process.send_signal(getattr(signal,mode))
            stdout,stderr=process.communicate(timeout=20)
            assert process.returncode==0,(mode,process.returncode,(stdout+stderr)[-4000:])
            assert 'CONTROLLER_AND_CLEANUP_PASSED:'+mode in stdout
            if child_pid:
                assert subprocess.run(['ps','-p',str(child_pid)],stdout=subprocess.DEVNULL).returncode!=0,'Owned child survived cleanup'
            checks.append(dict(mode=mode,exitCode=process.returncode,ownedChildStopped=bool(child_pid),
                repeatedSignalIgnored=bool(child_pid),approvalClosed=True,runDirectoryRemoved=True,
                ownedDataCacheGeneratedRemoved=True,previousSignalHandlersRestored=True))
        finally:
            if process.poll() is None:
                process.terminate()
                try:process.communicate(timeout=5)
                except subprocess.TimeoutExpired:process.kill();process.communicate()
            if child_pid:
                current=subprocess.run(['ps','-p',str(child_pid),'-o','command='],capture_output=True,text=True)
                if str(case/'child-ready') in current.stdout:
                    os.killpg(child_pid,signal.SIGKILL)
    footprint=sum(p.stat().st_size for p in folder.rglob('*') if p.is_file())
    assert footprint<1024**2
    subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py'),'--extra-root',temporary],check=True)
report={'atUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'scope':'Actual file controller plus SIGTERM/SIGHUP/SIGINT during a real owned subprocess, with DEVNULL stdin. Builds/snapshot are stubs; no native app or product acceptance.',
    'exitCode':0,'commands':['normal','snapshot','finish'],'checks':checks,
    'approvalClosed':True,'innerRunDirectoryRemoved':True,'emptyRegistryBootstrapPassed':True,'rustcAvailableInChildPath':True,
    'allTemporaryDataRemoved':not folder.exists(),'observedLogicalBytes':footprint,
    'controllerSourceSha256':hashlib.sha256((root/'scripts/exit-confirmation-batch.py').read_bytes()).hexdigest()}
(root/'docs/native-batch-control-check.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py')],check=True)
print(json.dumps(report,ensure_ascii=False))
