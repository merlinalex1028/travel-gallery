"""Compile only the real publish module and its boundary tests in a disposable crate."""
from pathlib import Path
import argparse
import datetime
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import tarfile
import urllib.request

ROOT=Path(__file__).resolve().parent.parent
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--expect-failure',action='store_true')
parser.add_argument('--windows-apis',action='store_true')
parser.add_argument('--report',type=Path,default=ROOT/'docs/publish-boundary-latest.json')
args=parser.parse_args()
report_path=args.report.absolute();assert report_path.parent==ROOT/'docs'
limits=(2000,2*1024**3)
subprocess.run([sys.executable,str(ROOT/'scripts/check-test-artifact-budget.py'),'--planned-files','1900','--planned-bytes',str(700*1024**2)],check=True)

def measure(root):
    count=size=0
    for base,dirs,files in os.walk(root,followlinks=False):
        for name in files:
            try:s=(Path(base)/name).lstat();count+=1;size+=s.st_size
            except FileNotFoundError:pass
        for name in dirs:
            p=Path(base)/name
            if p.is_symlink():count+=1;size+=p.lstat().st_size
    return count,size

packages=tomllib.loads((ROOT/'src-tauri/Cargo.lock').read_text())['package']
selected=set();queue=[('cap-std','3.4.6'),('rustix','1.1.5')]
if args.windows_apis:queue.append(('windows-sys','0.61.2'))
while queue:
    name,version=queue.pop()
    for p in packages:
        key=(p['name'],p['version'])
        if p['name']!=name or (version and p['version']!=version) or key in selected:continue
        selected.add(key)
        for dep in p.get('dependencies',[]):
            parts=dep.split();queue.append((parts[0],parts[1] if len(parts)>1 and parts[1][0].isdigit() else None))
result=None;peak=[0,0];tail=bytearray()
with tempfile.TemporaryDirectory(prefix='travel-gallery-publish-check-') as temporary:
    work=Path(temporary);home=work/'cargo-home';registry=home/'registry';registry.mkdir(parents=True)
    global_registry=Path.home()/'.cargo/registry'
    if (global_registry/'index').is_dir():shutil.copytree(global_registry/'index',registry/'index')
    for group in ((global_registry/'src').iterdir() if (global_registry/'src').is_dir() else []):
        dest=registry/'src'/group.name;dest.mkdir(parents=True)
        for name,version in selected:
            package=group/f'{name}-{version}'
            if (package/'.cargo-ok').is_file():(dest/package.name).symlink_to(package,target_is_directory=True)
    if not (global_registry/'src').is_dir() and not args.windows_apis:
        # Inspect the exact pinned host archives before Cargo may expand them.
        names={'ambient-authority','bitflags','cap-primitives','cap-std','errno','fs-set-times',
               'io-extras','io-lifetimes','ipnet','libc','maybe-owned','rustix'}
        cache=registry/'cache/index.crates.io-1949cf8c6b5b557f';cache.mkdir(parents=True)
        source_files=source_bytes=0
        for package in packages:
            if package['name'] not in names or (package['name'],package['version']) not in selected:continue
            archive=cache/f'{package["name"]}-{package["version"]}.crate'
            url=f'https://static.crates.io/crates/{package["name"]}/{archive.name}'
            with urllib.request.urlopen(url,timeout=20) as response,archive.open('wb') as output:
                while chunk:=response.read(65536):output.write(chunk)
            assert hashlib.sha256(archive.read_bytes()).hexdigest()==package['checksum']
            with tarfile.open(archive,'r:gz') as source:
                for entry in source:
                    if entry.isfile():source_files+=1;source_bytes+=entry.size
        assert source_files+600<=1900,f'Cold publication source requires {source_files} files plus compiler/metadata reserve'
        assert source_bytes+sum(p.stat().st_size for p in cache.iterdir())+128*1024**2<=700*1024**2
        print(json.dumps({'coldSourceFiles':source_files,'sourceLogicalBytes':source_bytes,'reservedCompilerAndMetadataFiles':600}),flush=True)
    (work/'src').mkdir();(work/'cases').mkdir()
    library=f'#[path={json.dumps(str(ROOT / "src-tauri/src/publish.rs"))}] pub mod publish;\n'
    library+=f'#[path={json.dumps(str(ROOT / "src-tauri/src/export_errors.rs"))}] pub mod export_errors;\n'
    if args.windows_apis:
        source=(ROOT/'src-tauri/src/files.rs').read_text()
        file_identity=source[source.index('pub fn identity('):source.index('pub fn source_path(')]
        library+='#[allow(unused_imports)] use std::{path::Path,fs::{self,File}};\n'
        library+=f'#[path={json.dumps(str(ROOT / "src-tauri/src/platform_power.rs"))}] pub mod platform_power;\n'+file_identity
    (work/'src/lib.rs').write_text(library)
    (work/'Cargo.toml').write_text('[package]\nname="travel-gallery-publish-check"\nversion="0.0.0"\nedition="2021"\n[lib]\nname="travel_gallery"\n[dependencies]\ncap-std="=3.4.6"\n[target.\'cfg(unix)\'.dependencies]\nrustix={version="=1.1.5",features=["fs"]}\n[target.\'cfg(windows)\'.dependencies]\nwindows-sys={version="=0.61.2",features=["Win32_Foundation","Win32_Storage_FileSystem","Win32_System_Power"]}\n[[test]]\nname="publish_boundaries"\npath='+json.dumps(str(ROOT/'src-tauri/tests/publish_boundaries.rs'))+'\n')
    shutil.copy2(ROOT/'src-tauri/Cargo.lock',work/'Cargo.lock')
    env=os.environ.copy();env.update(CARGO_HOME=str(home),CARGO_TARGET_DIR=str(work/'target'),CARGO_INCREMENTAL='0',CARGO_PROFILE_DEV_DEBUG='0',CARGO_PROFILE_TEST_DEBUG='0',TMPDIR=str(work/'cases'),PATH=str(Path.home()/'.cargo/bin')+os.pathsep+os.environ.get('PATH',''))
    command=[str(Path.home()/'.cargo/bin/cargo'),'test','--manifest-path',str(work/'Cargo.toml'),'--test','publish_boundaries','--','--test-threads=1']
    if args.windows_apis:command=[str(Path.home()/'.cargo/bin/cargo'),'check','--manifest-path',str(work/'Cargo.toml'),'--target','x86_64-pc-windows-gnu']
    child=subprocess.Popen(command,cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,start_new_session=True)
    def read():
        while True:
            chunk=child.stdout.read(8192)
            if not chunk:break
            tail.extend(chunk)
            if len(tail)>24000:del tail[:-24000]
    reader=threading.Thread(target=read,daemon=True);reader.start()
    try:
        while child.poll() is None:
            files,logical=measure(work);peak[0]=max(peak[0],files);peak[1]=max(peak[1],logical)
            if files>limits[0] or logical>limits[1]:raise RuntimeError(f'Default budget exceeded: {files} files, {logical} bytes')
            time.sleep(.25)
        reader.join(timeout=3);code=child.returncode
        final=measure(work);peak=[max(peak[i],final[i]) for i in range(2)]
        assert not list((work/'cases').iterdir()),'A filesystem fixture leaked'
        subprocess.run([sys.executable,str(ROOT/'scripts/check-test-artifact-budget.py'),'--extra-root',temporary],check=True)
        result=dict(atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(),scope='Real publish.rs and export_errors.rs with cap-std/rustix; filesystem and target-error boundary tests, not GUI or full Catalog export acceptance.',
            expectedFailure=args.expect_failure,exitCode=code,outputTail=tail.decode(errors='replace')[-12000:],
            publishSha256=hashlib.sha256((ROOT/'src-tauri/src/publish.rs').read_bytes()).hexdigest(),testSha256=hashlib.sha256((ROOT/'src-tauri/tests/publish_boundaries.rs').read_bytes()).hexdigest(),
            errorPolicySha256=hashlib.sha256((ROOT/'src-tauri/src/export_errors.rs').read_bytes()).hexdigest(),
            peakFiles=peak[0],peakLogicalBytes=peak[1],sourceMediaFiles=0,fixturesRemoved=True)
        if args.windows_apis:result['scope']='x86_64-pc-windows-gnu type check of production publish/power modules and source identity function; no Windows process, installer or full Catalog build.'
    finally:
        if child.poll() is None:
            os.killpg(child.pid,signal.SIGTERM)
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait()
assert result is not None
result['temporaryDirectoryRemoved']=not Path(temporary).exists();assert result['temporaryDirectoryRemoved']
report_path.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
subprocess.run([sys.executable,str(ROOT/'scripts/check-test-artifact-budget.py')],check=True)
print(json.dumps(result,ensure_ascii=False))
valid_result=code==0 if not args.expect_failure else code==101 and 'test result: FAILED.' in result['outputTail']
raise SystemExit(0 if valid_result else 1)
