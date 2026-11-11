"""Native ENOSPC fixture within the default budget; reuse the accepted binary, no builds."""
from pathlib import Path
import errno
import hashlib
import importlib.util
import json
import os
import plistlib
import re
import shutil
import sqlite3
import struct
import subprocess
import sys
import tempfile
import time
import zlib

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('batch', ROOT/'scripts/exit-confirmation-batch.py')
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)
b.APPROVAL = ROOT/'docs/native-disk-full-default-budget.json'
b.RECEIPT = ROOT/'docs/native-disk-full-latest.json'
b.DATA = b.LIBRARY/'Application Support/com.travelgallery.desktop'
b.CACHE = b.LIBRARY/'Caches/com.travelgallery.desktop'
OWNED = [b.LIBRARY/sub/identifier for sub in ('Application Support', 'Caches')
         for identifier in ('com.travelgallery.desktop', b.IDENTIFIER)]
cleanup_spec = importlib.util.spec_from_file_location('cleanup', ROOT/'scripts/cleanup-test-artifacts.py')
cleanup = importlib.util.module_from_spec(cleanup_spec)
cleanup_spec.loader.exec_module(cleanup)
b.EXTRA = [path for path in cleanup.qa_namespace_paths(Path.home(), b.IDENTIFIER) if path not in OWNED]
TABLES = ('asset_reviews', 'tags', 'asset_tags', 'linked_media', 'collections',
          'collection_assets', 'saved_filters', 'source_options', 'app_settings')
SCOPE = 'Default-budget production-binary native disk-full acceptance'
RUN_PREFIX = 'travel-gallery-native-disk-full-'
PLANNED_BYTES = 1536*1024**2
PLANNED_FILES = 250
SOURCE_FILES = 5
SANDBOX_SEAL_ARGS = ('--webkit-mach-lookup',)


def write_sources(source):
    def chunk(kind, payload):
        return struct.pack('>I', len(payload))+kind+payload+struct.pack('>I', zlib.crc32(kind+payload)&0xffffffff)
    for index in range(5):
        data = b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0))+chunk(b'IDAT', zlib.compress(bytes([0,index*40,80,120,255])))+chunk(b'IEND', b'')
        (source/f'owned-{index}.png').write_bytes(data)


def normal_state():
    library = Path.home()/'Library'
    data = library/'Application Support/com.travelgallery.desktop'
    cache = library/'Caches/com.travelgallery.desktop'
    path = data/'catalog.sqlite'
    if not path.exists() and not path.is_symlink():
        return dict(media=0, tables={}, catalogPresent=False,
                    dataDirectoryPresent=data.exists() or data.is_symlink(),
                    cacheDirectoryPresent=cache.exists() or cache.is_symlink())
    with sqlite3.connect(f'file:{path}?mode=ro', uri=True) as conn:
        conn.execute('BEGIN')
        tables = {table: hashlib.sha256('\n'.join(sorted(json.dumps(tuple(row), ensure_ascii=False, sort_keys=True)
            for row in conn.execute(f'SELECT * FROM {table}'))).encode()).hexdigest() for table in TABLES}
        return dict(media=conn.execute('SELECT COUNT(*) FROM media_assets').fetchone()[0], tables=tables)


def source_state():
    source = b.WORK/'fixture/source'
    assert not source.is_symlink() and not source.parent.is_symlink(), 'Owned source inventory must not follow links'
    paths = sorted(source.rglob('*'))
    assert not any(p.is_symlink() for p in paths), 'Owned source inventory must not follow links'
    return [dict(name=p.relative_to(source).as_posix(), sha256=hashlib.sha256(p.read_bytes()).hexdigest(), bytes=p.stat().st_size,
        mtimeNs=p.stat().st_mtime_ns, inode=p.stat().st_ino, links=p.stat().st_nlink)
        for p in paths if p.is_file()]


def owned_images():
    info = plistlib.loads(subprocess.check_output(['hdiutil', 'info', '-plist']))
    image = (b.WORK/'disk-full.dmg').resolve()
    return [item for item in info.get('images', []) if item.get('image-path') and
            Path(item['image-path']).resolve() == image]


class RunDirectory:
    def __init__(self):
        self.inner = tempfile.TemporaryDirectory(prefix=RUN_PREFIX)
        self.name = self.inner.name
        # Never let GC recursively delete a directory while its test image is mounted.
        self.inner._finalizer.detach()

    def cleanup(self):
        if 'sourceBefore' in b.STATE and (b.WORK/'fixture/source').exists():
            b.STATE['sourceAfter'] = source_state()
            b.STATE['sourceUnchanged'] = b.STATE['sourceBefore'] == b.STATE['sourceAfter']
        b.STATE['normalAfter'] = normal_state()
        b.STATE['normalPreserved'] = b.STATE['normalBefore'] == b.STATE['normalAfter']
        b.save()
        for image in owned_images():
            devices = [p['dev-entry'] for p in image.get('system-entities', [])
                       if re.fullmatch(r'/dev/disk\d+', p.get('dev-entry', ''))]
            assert devices, 'Cannot identify owned image device; retaining directory'
            device = devices[0]
            result = subprocess.run(['hdiutil', 'detach', device], capture_output=True, text=True)
            if result.returncode:
                subprocess.run(['hdiutil', 'detach', '-force', device], check=True, capture_output=True)
        assert not owned_images(), 'Owned image still attached; retaining directory'
        assert not os.path.ismount(b.WORK/'fixture/volume'), 'Refusing to delete a mounted volume'
        b.STATE['ownedImageDetached'] = True
        b.save()
        self.inner.cleanup()


def prepare_volume():
    image = b.WORK/'disk-full.dmg'
    volume = b.WORK/'fixture/volume'
    assert not image.exists() and not volume.exists()
    b.command('createOwnedImage', ['hdiutil', 'create', '-size', '512m', '-fs', 'APFS',
        '-volname', 'TravelGalleryDisposableQA', '-type', 'UDIF', '-nospotlight', str(image)])
    volume.mkdir()
    b.command('attachOwnedImage', ['hdiutil', 'attach', '-nobrowse', '-mountpoint', str(volume), str(image)])
    assert os.path.ismount(volume) and len(owned_images()) == 1
    (volume/'output').mkdir()
    b.event('ownedVolumeReady', mount=str(volume), output=str(volume/'output'), freeBytes=shutil.disk_usage(volume).free, **b.check())


def fill_volume():
    volume = b.WORK/'fixture/volume'
    assert os.path.ismount(volume) and len(owned_images()) == 1
    assert shutil.disk_usage(volume).total <= 512*1024**2, 'Refusing to fill a larger filesystem'
    filler = volume/'owned-filler.bin'
    assert not filler.exists()
    observed = None
    with filler.open('wb', buffering=0) as stream:
        while True:
            try:
                stream.write(b'\x5a'*(1024*1024))
                os.fsync(stream.fileno())
                b.check()
            except OSError as error:
                if error.errno != errno.ENOSPC:
                    raise
                observed = error.errno
                break
    # Large-file allocation can fail while a new small file still fits on APFS.
    residual = []
    for index in range(32):
        path = volume/f'owned-residual-{index}.bin'
        assert not path.exists()
        try:
            with path.open('xb', buffering=0) as stream:
                for _ in range(8192):
                    stream.write(b'R'*4096)
                    os.fsync(stream.fileno())
                    b.check()
        except OSError as error:
            if error.errno != errno.ENOSPC:
                raise
        else:
            raise RuntimeError('Residual allocation did not reach ENOSPC within its bound')
        size = path.stat().st_size if path.exists() else 0
        residual.append(dict(index=index, bytes=size, errno=errno.ENOSPC))
        if size == 0:
            break
    assert residual[-1]['bytes'] == 0, 'Small-file exhaustion not established'
    b.event('ownedVolumeFull', errno=observed, fillerBytes=filler.stat().st_size, residualFiles=residual,
            freeBytes=shutil.disk_usage(volume).free, **b.check())


def snapshot(label):
    database = b.DATA/'catalog.sqlite'
    with sqlite3.connect(f'file:{database}?mode=ro', uri=True) as conn:
        conn.row_factory = sqlite3.Row
        media = conn.execute('SELECT COUNT(*) FROM media_assets').fetchone()[0]
        jobs = [dict(r) for r in conn.execute('SELECT id,type,status,progress_done,progress_total,failure_count FROM jobs ORDER BY created_at')]
        items = [dict(r) for r in conn.execute('SELECT job_id,dest_relative_path,state,copied_bytes,error_code FROM export_run_items')]
    b.event('nativeSnapshot', label=label, database=str(database), media=media, jobs=jobs, items=items,
            sourceUnchanged=source_state() == b.STATE['sourceBefore'], normalPreserved=normal_state() == b.STATE['normalBefore'], **b.check())


def main():
    assert not any(p.exists() or p.is_symlink() for p in [*OWNED, *b.EXTRA]), 'QA namespace is not empty'
    subprocess.run([sys.executable, str(ROOT/'scripts/check-test-artifact-budget.py'),
        '--planned-source-files', str(SOURCE_FILES), '--planned-files', str(PLANNED_FILES), '--planned-bytes', str(PLANNED_BYTES)], check=True)
    b.STATE['normalBefore'] = normal_state()
    info = json.loads((ROOT/'artifacts/build-info.json').read_bytes())
    main_executable = b.NORMAL/'Contents/MacOS/travel-gallery'
    assert hashlib.sha256(main_executable.read_bytes()).hexdigest() == info['executableSha256']
    b.STATE['acceptedRelease'] = dict(zipSha256=info['sha256'], executableSha256=info['executableSha256'], reuse='Unchanged accepted executable before QA signing; no compilation')
    b.TEMP = RunDirectory()
    b.WORK = Path(b.TEMP.name)
    b.STATE['temporaryRoot'] = str(b.WORK)
    b.save()
    fixture = b.WORK/'fixture'
    source = fixture/'source'
    source.mkdir(parents=True)
    write_sources(source)
    b.STATE['sourceBefore'] = source_state()
    marker = dict(identifier=b.IDENTIFIER, count=SOURCE_FILES, fixtureRoot=str(source), scope=SCOPE)
    (fixture/'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
    stage = b.WORK/'unsigned/Travel Gallery Offline Bench.app'
    stage.parent.mkdir()
    b.command('copyAcceptedApp', ['ditto', str(b.NORMAL), str(stage)])
    path = stage/'Contents/Info.plist'
    plist = plistlib.loads(path.read_bytes())
    plist.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName='Travel Gallery Offline Bench', CFBundleDisplayName='Travel Gallery Offline Bench')
    path.write_bytes(plistlib.dumps(plist))
    b.command('sealIsolatedCopy', [sys.executable, 'scripts/prepare-offline-sandbox.py', '--source', str(stage),
        '--fixture', str(fixture), '--output', str(b.WORK/'qa'), *SANDBOX_SEAL_ARGS])
    shutil.rmtree(stage.parent)
    b.APP_PATH = b.WORK/'qa/Travel Gallery Offline Bench.app'
    for path in OWNED:
        b.OWNED_PATHS.append(path)
    b.STATE.update(application=str(b.APP_PATH), status='ready', expectedDatabase=str(b.DATA/'catalog.sqlite'))
    control = b.WORK/'control'
    control.mkdir()
    b.event('readyForNativeGui', application=str(b.APP_PATH), source=str(source), controlDirectory=str(control), **b.check())
    while True:
        b.check()
        for path in sorted(control.glob('*.json')):
            request = json.loads(path.read_bytes()); path.unlink()
            op = request['op']
            if op == 'volume': prepare_volume()
            elif op == 'fill': fill_volume()
            elif op == 'release':
                (fixture/'volume/owned-filler.bin').unlink()
                b.event('spaceReleased', freeBytes=shutil.disk_usage(fixture/'volume').free, **b.check())
            elif op == 'snapshot': snapshot(request['label'])
            elif op == 'finish': b.STATE['status'] = 'finished'; return
            else: raise ValueError(f'Unknown command: {op}')
        time.sleep(.25)


def run():
    if b.APPROVAL.exists():
        assert json.loads(b.APPROVAL.read_bytes())['status'] == 'closed', 'Previous default-budget batch is still registered'
    registration = dict(status='active', authorization='AGENTS.md default budget; no elevated user approval claimed',
        scope=SCOPE+'; stop/delete on completion, failure or interruption',
        limits=dict(files=2000, logicalBytes=2*1024**3, sourceFiles=SOURCE_FILES))
    b.APPROVAL.write_text(json.dumps(registration, indent=2)+'\n')
    b.main = main
    return b.run()


if __name__ == '__main__':
    raise SystemExit(run())
