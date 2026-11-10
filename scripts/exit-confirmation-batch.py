"""One approved, monitored native batch. Atomic JSON command files; no GUI automation."""
from pathlib import Path
import datetime
import hashlib
import json
import os
import plistlib
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import uuid
import struct
import zlib

ROOT=Path(__file__).resolve().parent.parent
CURRENT_ARCHIVE=json.loads((ROOT/'artifacts/build-info.json').read_text())['file']
APPROVAL=ROOT/'docs/exit-confirmation-budget-20261031.json'
RECEIPT=ROOT/'docs/exit-confirmation-batch-latest.json'
IDENTIFIER='com.travelgallery.offlineaudit'
NORMAL=ROOT/'src-tauri/target/release/bundle/macos/Travel Gallery.app'
LIBRARY=Path.home()/'Library/Containers'/IDENTIFIER/'Data/Library'
DATA=LIBRARY/'Application Support'/IDENTIFIER
CACHE=LIBRARY/'Caches'/IDENTIFIER
EXTRA=[LIBRARY/'WebKit'/IDENTIFIER,LIBRARY/'HTTPStorages'/IDENTIFIER]
STATE=dict(startedAt=datetime.datetime.now(datetime.timezone.utc).isoformat(),status='starting',events=[],peakFiles=0,peakLogicalBytes=0)
STOP=threading.Event()
CHILD=None
WORK=None
APP_PATH=None
TEMP=None
OWNED_PATHS=[]
GENERATED=ROOT/'src-tauri/gen'
OWNS_GENERATED=False
WINDOW_TITLE='旅图 · 退出确认小夹具'
TEMP_PREFIX='travel-gallery-exit-batch-'
FIXTURE_SCOPE='Owned five tiny PNGs plus controlled paused SQL job; exit confirmation only'
PLANNED_SOURCE_FILES=5
PLANNED_FILES=18000
INTERRUPTION_SIGNALS=(signal.SIGINT,signal.SIGTERM,signal.SIGHUP)

def regressions(env):
    """Optional sequential checks before creating the GUI dataset."""
    pass

def save():
    p=RECEIPT.with_suffix('.tmp');p.write_text(json.dumps(STATE,ensure_ascii=False,indent=2)+'\n');p.replace(RECEIPT)

def event(stage,**details):
    row=dict(atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),stage=stage,**details)
    STATE['events'].append(row);save();print(json.dumps(row,ensure_ascii=False),flush=True)

def footprint(path):
    count=size=0
    if path.is_symlink():return 1,path.lstat().st_size
    if not path.exists():return count,size
    if path.is_file():return 1,path.lstat().st_size
    for base,dirs,files in os.walk(path,followlinks=False):
        for name in files:
            p=Path(base)/name
            try:s=p.lstat();count+=1;size+=s.st_size
            except FileNotFoundError:pass
        for name in dirs:
            p=Path(base)/name
            if p.is_symlink():count+=1;size+=p.lstat().st_size
    return count,size

def check():
    a=json.loads(APPROVAL.read_text());assert a['status']=='active'
    retained={'build-info.json','SHA256SUMS.txt',CURRENT_ARCHIVE}
    temporary_artifacts=[p for p in (ROOT/'artifacts').iterdir() if p.name not in retained]
    stats=[footprint(p) for p in [WORK,DATA,CACHE,GENERATED,*EXTRA,*temporary_artifacts] if p]
    count=sum(x[0] for x in stats);size=sum(x[1] for x in stats)
    STATE['peakFiles']=max(STATE['peakFiles'],count);STATE['peakLogicalBytes']=max(STATE['peakLogicalBytes'],size)
    if count>a['limits']['files'] or size>a['limits']['logicalBytes']:
        raise RuntimeError(f'Approved resource budget exceeded: {count} files / {size} logical bytes')
    return dict(files=count,logicalBytes=size)

def command(stage,args,env=None):
    global CHILD
    check();event(stage+'Started',command=args)
    CHILD=subprocess.Popen(args,cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,start_new_session=True)
    tail=bytearray()
    def read():
        while True:
            chunk=CHILD.stdout.read(8192)
            if not chunk:break
            tail.extend(chunk)
            if len(tail)>65536:del tail[:-65536]
    reader=threading.Thread(target=read,daemon=True);reader.start()
    last=time.monotonic()
    try:
        while CHILD.poll() is None:
            stats=check()
            if time.monotonic()-last>20:
                print(json.dumps(dict(stage=stage+'Progress',**stats)),flush=True);last=time.monotonic()
            time.sleep(1)
        reader.join(timeout=3);code=CHILD.returncode
    finally:
        if CHILD.poll() is None:
            os.killpg(CHILD.pid,signal.SIGTERM)
            try:CHILD.wait(timeout=10)
            except subprocess.TimeoutExpired:os.killpg(CHILD.pid,signal.SIGKILL);CHILD.wait()
    CHILD=None
    event(stage+'Finished',exitCode=code,outputTail=tail.decode(errors='replace')[-12000:],**check())
    if code:raise RuntimeError(f'{stage} failed with exit {code}')

def prepare_home(registry_source=None):
    home=WORK/'cargo-home';home.mkdir()
    global_registry=Path(registry_source) if registry_source is not None else Path.home()/'.cargo/registry';registry=home/'registry';registry.mkdir()
    if (global_registry/'index').is_dir():shutil.copytree(global_registry/'index',registry/'index')
    # Reuse only complete existing source packages; newly downloaded archives stay here.
    for group in ((global_registry/'src').iterdir() if (global_registry/'src').is_dir() else []):
        dest=registry/'src'/group.name;dest.mkdir(parents=True)
        for package in group.iterdir():
            if package.is_dir() and (package/'.cargo-ok').is_file():(dest/package.name).symlink_to(package,target_is_directory=True)
    env=os.environ.copy();env.update(CARGO_HOME=str(home),CARGO_TARGET_DIR=str(WORK/'target'),CARGO_INCREMENTAL='0',CARGO_PROFILE_RELEASE_DEBUG='0',CARGO_PROFILE_RELEASE_STRIP='debuginfo',PATH=str(Path.home()/'.cargo/bin')+os.pathsep+os.environ.get('PATH',''))
    return env

def frontend(mode,env):
    path=WORK/('frontend-'+mode)
    command('frontendTypes',['pnpm','exec','vue-tsc','--noEmit'],env)
    command('frontend'+mode,['pnpm','exec','vite','build','--mode',mode,'--outDir',str(path)],env)
    return path

def build(qa,env):
    front=frontend('performance' if qa else 'production',env)
    config=dict(build=dict(frontendDist=str(front)))
    if qa:config.update(productName='Travel Gallery Offline Bench',identifier=IDENTIFIER,app=dict(windows=[dict(label='main',title=WINDOW_TITLE,width=1440,height=920,minWidth=860,minHeight=600,alwaysOnTop=False,visibleOnAllWorkspaces=False)]))
    build_env={**env,'TAURI_CONFIG':json.dumps(config,ensure_ascii=False)}
    features='offline-network-probe,custom-protocol' if qa else 'desktop,custom-protocol'
    command('qaCompile' if qa else 'normalCompile',['node','scripts/native.mjs','cargo','build','--release','--locked','--manifest-path','src-tauri/Cargo.toml','--features',features,'--bin','travel-gallery'],build_env)
    stage=WORK/('qa-unsigned' if qa else 'normal-candidate');stage.mkdir()
    app=stage/('Travel Gallery Offline Bench.app' if qa else 'Travel Gallery.app')
    command('copyApp',['ditto',str(NORMAL),str(app)],env)
    shutil.copy2(WORK/'target/release/travel-gallery',app/'Contents/MacOS/travel-gallery')
    if qa:
        p=app/'Contents/Info.plist';info=plistlib.loads(p.read_bytes());info.update(CFBundleIdentifier=IDENTIFIER,CFBundleName='Travel Gallery Offline Bench',CFBundleDisplayName='Travel Gallery Offline Bench');p.write_bytes(plistlib.dumps(info))
        command('sealQA',['python3','scripts/prepare-offline-sandbox.py','--source',str(app),'--fixture',str(WORK/'fixture'),'--output',str(WORK/'qa'),'--webkit-mach-lookup'],env)
        shutil.rmtree(stage)
        return WORK/'qa/Travel Gallery Offline Bench.app'
    command('sealNormal',['codesign','--force','--sign','-','--timestamp=none',str(app)],env)
    command('verifyNormal',['codesign','--verify','--deep','--strict',str(app)],env)
    return app

def seed():
    assert APP_PATH and DATA.exists()
    c=sqlite3.connect(DATA/'catalog.sqlite');c.execute('PRAGMA foreign_keys=ON')
    assert not c.execute("select id from jobs where status in ('queued','running','paused')").fetchall()
    sid=str(uuid.uuid4());jid=str(uuid.uuid4());now=int(time.time()*1000)
    c.execute('insert into source_roots(id,absolute_path,display_name,created_at,updated_at) values(?,?,?,?,?)',(sid,str(WORK/'fixture/source'),'退出确认受控来源',now,now))
    c.execute('insert into source_options(source_id,options_json) values(?,?)',(sid,'{}'))
    c.execute("insert into jobs(id,type,status,progress_done,progress_total,checkpoint_json,created_at,updated_at,total_is_known) values(?,'SCAN','paused',0,5,?,?,?,1)",(jid,json.dumps(dict(sourceId=sid)),now,now))
    c.commit();c.close();STATE['seededJobId']=jid;STATE['seededSourceId']=sid
    event('controlledFixtureSeeded',jobId=jid,sourceId=sid,scope='SQL fixture for deterministic pending-work close guard; not native scan/pause acceptance')

def snapshot(label):
    c=sqlite3.connect(f'file:{DATA / "catalog.sqlite"}?mode=ro',uri=True);c.row_factory=sqlite3.Row
    jobs=[dict(r) for r in c.execute('select * from jobs order by created_at,id')]
    count=c.execute('select count(*) from media_assets').fetchone()[0];c.close()
    traces=[]
    for p in (DATA/'performance').glob('*.jsonl'):
        for line in p.open():
            try:r=json.loads(line)
            except json.JSONDecodeError:continue
            if r.get('type') in ('nativeStart','nativeLifecycleEvent'):traces.append(r)
    control=DATA/'offline-process-control.json'
    event('snapshot',label=label,mediaCount=count,jobs=jobs,nativeLifecycle=traces,parentControl=json.loads(control.read_text()) if control.exists() else None,**check())

def control_loop(env):
    control=WORK/'control';control.mkdir(exist_ok=True)
    STATE['controlDirectory']=str(control);event('controlReady',path=str(control))
    while True:
        check()
        for path in sorted(control.glob('*.json')):
            request=json.loads(path.read_text());path.unlink();op=request['op']
            if op=='seed':seed()
            elif op=='snapshot':snapshot(request['label'])
            elif op=='normal':STATE['normalCandidate']=str(build(False,env));event('normalCandidateReady',path=STATE['normalCandidate'])
            elif op=='finish':STATE['status']='finished';return
            else:raise RuntimeError(f'Unknown batch command {op}')
        time.sleep(1)

def main():
    global WORK,APP_PATH,TEMP,OWNS_GENERATED
    approval=json.loads(APPROVAL.read_text());assert approval['status']=='active'
    assert not DATA.exists() and not CACHE.exists(),'Owned QA namespace must be absent'
    assert not GENERATED.exists() and not GENERATED.is_symlink(),'Clean previous generated schemas before this batch'
    OWNS_GENERATED=True
    subprocess.run([sys.executable,str(ROOT/'scripts/check-test-artifact-budget.py'),'--approval-file',str(APPROVAL),'--planned-files',str(PLANNED_FILES),'--planned-bytes',str(5*1024**3),'--planned-source-files',str(PLANNED_SOURCE_FILES)],check=True)
    TEMP=tempfile.TemporaryDirectory(prefix=TEMP_PREFIX)
    WORK=Path(TEMP.name);STATE['temporaryRoot']=str(WORK);save()
    (WORK/'cases').mkdir()
    env=prepare_home();env.update(TMPDIR=str(WORK/'cases'),CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0')
    regressions(env)
    fixture=WORK/'fixture';source=fixture/'source';source.mkdir(parents=True)
    def chunk(kind,payload):return struct.pack('>I',len(payload))+kind+payload+struct.pack('>I',zlib.crc32(kind+payload)&0xffffffff)
    for i in range(5):
        png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',1,1,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(bytes([0,i*50,60,120,255])))+chunk(b'IEND',b'')
        (source/f'owned-{i}.png').write_bytes(png)
    STATE['sourceManifest']=[dict(name=p.name,bytes=p.stat().st_size,sha256=hashlib.sha256(p.read_bytes()).hexdigest()) for p in sorted(source.iterdir())]
    marker=dict(identifier=IDENTIFIER,count=5,fixtureRoot=str(source),scope=FIXTURE_SCOPE)
    (fixture/'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    APP_PATH=build(True,env)
    for p in [DATA,CACHE]:
        p.mkdir(parents=True);OWNED_PATHS.append(p);(p/'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    STATE['application']=str(APP_PATH);STATE['status']='ready';event('readyForNativeGui',application=str(APP_PATH),**check())
    control_loop(env)

def _run_with_cleanup():
    try:main()
    except BaseException as error:
        STATE['status']='failed';STATE['error']=str(error);print(str(error),flush=True)
    finally:
        # A second interrupt must not abandon child shutdown or directory removal.
        for signum in INTERRUPTION_SIGNALS:signal.signal(signum,signal.SIG_IGN)
        # The controller must end the QA App before sending finish. Only our exact path may be stopped on failure.
        temporary_apps=[p for p in (APP_PATH,Path(STATE['normalCandidate']) if STATE.get('normalCandidate') else None) if p]
        for temporary_app in temporary_apps:
            for line in subprocess.check_output(['ps','-axo','pid=,comm='],text=True).splitlines():
                parts=line.strip().split(None,1)
                if len(parts)==2 and parts[1]==str((temporary_app/'Contents/MacOS/travel-gallery').resolve()):
                    STATE.setdefault('forcedCleanupPids',[]).append(int(parts[0]))
                    try:os.kill(int(parts[0]),signal.SIGTERM)
                    except ProcessLookupError:continue
                    for _ in range(30):
                        if subprocess.run(['ps','-p',parts[0]],stdout=subprocess.DEVNULL).returncode:break
                        time.sleep(.1)
                    else:
                        current=subprocess.run(['ps','-p',parts[0],'-o','comm='],capture_output=True,text=True)
                        if current.stdout.strip()==parts[1]:
                            try:os.kill(int(parts[0]),signal.SIGKILL)
                            except ProcessLookupError:pass
                            for _ in range(20):
                                if subprocess.run(['ps','-p',parts[0]],stdout=subprocess.DEVNULL).returncode:break
                                time.sleep(.1)
                            else:raise RuntimeError('Owned QA process did not stop; refusing to delete its live data')
        for p in [*OWNED_PATHS,*(EXTRA if OWNED_PATHS else [])]:
            if p.is_symlink():p.unlink()
            elif p.exists():shutil.rmtree(p)
        if OWNS_GENERATED:
            if GENERATED.is_symlink():GENERATED.unlink()
            elif GENERATED.exists():shutil.rmtree(GENERATED)
        if TEMP:TEMP.cleanup()
        STATE['temporaryDirectoryRemoved']=WORK is None or not WORK.exists()
        STATE['finishedAt']=datetime.datetime.now(datetime.timezone.utc).isoformat();save()
        approval=json.loads(APPROVAL.read_text());approval['status']='closed';approval['closedAt']=STATE['finishedAt'];APPROVAL.write_text(json.dumps(approval,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps(dict(stage='batchClosed',status=STATE['status'],temporaryDirectoryRemoved=STATE['temporaryDirectoryRemoved']),ensure_ascii=False),flush=True)
    return 0 if STATE['status']=='finished' else 1

def run():
    previous={}
    def interrupted(signum,frame):
        for pending in INTERRUPTION_SIGNALS:signal.signal(pending,signal.SIG_IGN)
        STATE['terminationSignal']=signal.Signals(signum).name
        raise InterruptedError('Batch interrupted by '+STATE['terminationSignal'])
    try:
        for signum in INTERRUPTION_SIGNALS:
            previous[signum]=signal.signal(signum,interrupted)
        return _run_with_cleanup()
    finally:
        for signum,handler in previous.items():signal.signal(signum,handler)

if __name__=='__main__':
    raise SystemExit(run())
