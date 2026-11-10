"""Measure locked desktop dependency archives without extracting them; remove all downloads."""
from pathlib import Path
import concurrent.futures
import datetime
import hashlib
import json
import subprocess
import sys
import tarfile
import tempfile
import threading
import tomllib
import urllib.request

root=Path(__file__).resolve().parent.parent
manifest=tomllib.loads((root/'src-tauri/Cargo.toml').read_text())
packages=tomllib.loads((root/'src-tauri/Cargo.lock').read_text())['package']
desktop={s.removeprefix('dep:') for s in manifest['features']['desktop']}
queue=[]
for section in (manifest['dependencies'],manifest['dev-dependencies'],manifest['build-dependencies'],
                *(v['dependencies'] for v in manifest['target'].values())):
    for name,options in section.items():
        if not isinstance(options,dict) or not options.get('optional') or name in desktop:queue.append((name,None))
selected={}
while queue:
    name,version=queue.pop()
    for package in packages:
        key=(package['name'],package['version'])
        if name!=package['name'] or version and version!=package['version'] or key in selected:continue
        selected[key]=package
        for dependency in package.get('dependencies',[]):
            parts=dependency.split();queue.append((parts[0],parts[1] if len(parts)>1 and parts[1][0].isdigit() else None))
selected=[p for p in selected.values() if p.get('source','').startswith('registry+')]
assert len(selected)<700
subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py'),
    '--planned-files','750','--planned-bytes',str(1024**3)],check=True)
lock=threading.Lock();downloaded=0;stop=threading.Event();results=[]
with tempfile.TemporaryDirectory(prefix='travel-gallery-cargo-forecast-') as temporary:
    folder=Path(temporary)
    def measure(package):
        global downloaded
        if stop.is_set():raise RuntimeError('Forecast stopped')
        name,version=package['name'],package['version'];path=folder/f'{name}-{version}.crate'
        url=f'https://static.crates.io/crates/{name}/{name}-{version}.crate'
        digest=hashlib.sha256();size=0
        with urllib.request.urlopen(url,timeout=20) as response,path.open('wb') as output:
            while chunk:=response.read(64*1024):
                if stop.is_set():raise RuntimeError('Forecast stopped')
                with lock:
                    downloaded+=len(chunk)
                    if downloaded>1024**3:stop.set();raise RuntimeError('Compressed forecast budget exceeded')
                output.write(chunk);digest.update(chunk);size+=len(chunk)
        assert digest.hexdigest()==package['checksum'],f'Checksum mismatch: {name}-{version}'
        files=logical=0
        with tarfile.open(path,'r:gz') as archive:
            for entry in archive:
                if entry.isfile():files+=1;logical+=entry.size
                elif entry.issym() or entry.islnk():files+=1;logical+=len(entry.linkname.encode())
        return {'package':f'{name}-{version}','archiveBytes':size,'sourceFiles':files,'sourceLogicalBytes':logical}
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futures=[pool.submit(measure,p) for p in selected]
        try:
            for future in concurrent.futures.as_completed(futures):
                results.append(future.result())
                if len(results)%50==0:print(json.dumps({'downloadedPackages':len(results),'totalPackages':len(selected),'archiveBytes':downloaded}),flush=True)
        except BaseException:
            stop.set()
            for future in futures:future.cancel()
            raise
    subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py'),'--extra-root',temporary],check=True)
assert not folder.exists()
report={'atUtc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'scope':'Locked desktop/core/dev transitive registry packages across all target platforms; conservative source upper bound, not the exact macOS compile set. Archives verified by Cargo.lock SHA; never extracted.',
    'packages':len(results),'archiveBytes':sum(x['archiveBytes'] for x in results),
    'unpackedSourceFiles':sum(x['sourceFiles'] for x in results),
    'unpackedSourceLogicalBytes':sum(x['sourceLogicalBytes'] for x in results),
    'largestFileCounts':sorted(results,key=lambda x:x['sourceFiles'],reverse=True)[:12],
    'downloadDirectoryRemoved':True}
(root/'docs/cold-cargo-forecast.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
subprocess.run([sys.executable,str(root/'scripts/check-test-artifact-budget.py')],check=True)
print(json.dumps(report,ensure_ascii=False))
