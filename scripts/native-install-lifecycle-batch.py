"""Install an isolated copy from the shipped ZIP, remove the App, and reinstall it."""
from contextlib import closing
import hashlib
import importlib.util
import json
from pathlib import Path
import plistlib
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('picker', Path(__file__).with_name('native-picker-restart-batch.py'))
picker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(picker)
reuse, b = picker.reuse, picker.b
b.APPROVAL = reuse.ROOT / 'docs/native-install-lifecycle-default-budget.json'
b.RECEIPT = reuse.ROOT / 'docs/native-install-lifecycle-latest.json'
NAME = 'Travel Gallery Offline Bench.app'
PROGRAMS = {
    'travel-gallery': 'executableSha256',
    'travel-gallery-ai': 'aiWorkerSha256',
    'travel-gallery-quality': 'qualityWorkerSha256',
    'ffmpeg': 'ffmpegSha256',
    'ffprobe': 'ffprobeSha256',
}
STEPS = ('before-import', 'before-remove', 'remove', 'reinstall', 'after-reinstall', 'finish')


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def programs(app):
    return {name: sha(app / 'Contents/MacOS' / name) for name in PROGRAMS}


def install(sealed):
    assert not b.APP_PATH.exists() and not b.APP_PATH.is_symlink()
    b.command('installOwnedApp', ['/usr/bin/ditto', str(sealed), str(b.APP_PATH)])
    subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(b.APP_PATH)], check=True)
    assert programs(b.APP_PATH) == programs(sealed)


def read_catalog(database, empty=False):
    with closing(sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)) as conn:
        conn.execute('BEGIN')
        assert conn.execute('PRAGMA quick_check').fetchone() == ('ok',)
        assert not conn.execute('PRAGMA foreign_key_check').fetchall()
        assert not conn.execute("SELECT 1 FROM jobs WHERE status IN ('queued','running','paused')").fetchone()
        roots = conn.execute('SELECT id,absolute_path,is_online FROM source_roots ORDER BY id').fetchall()
        media = conn.execute('SELECT id,relative_path,file_state FROM media_assets ORDER BY id').fetchall()
        reviews = conn.execute('SELECT * FROM asset_reviews ORDER BY asset_id').fetchall()
        if empty:
            assert not roots and not media and not reviews
            return None
        assert len(roots) == 1 and roots[0][1:] == (str((b.WORK / 'fixture/source').resolve()), 1)
        assert len(media) == len(reviews) == reuse.SOURCE_FILES
        assert all(state == 'available' for _, _, state in media)
        assert conn.execute("SELECT count(*) FROM asset_reviews WHERE rating=4 AND pick_state='keep'").fetchone()[0] == 1
        others = {table: sorted(conn.execute(f'SELECT * FROM {table}').fetchall())
                  for table in ('change_batches', 'asset_tags', 'tags', 'collections')}
        return hashlib.sha256(json.dumps([roots, media, reviews, others]).encode()).hexdigest()


def ensure_closed():
    executable = str((b.APP_PATH / 'Contents/MacOS/travel-gallery').resolve())
    processes = subprocess.check_output(['ps', '-axww', '-o', 'comm='], text=True, errors='replace')
    assert executable not in {line.strip() for line in processes.splitlines()}, 'Quit the isolated App first'


def persistent_files(database):
    thumbnails = b.CACHE / 'thumbs'
    assert thumbnails.is_dir() and not thumbnails.is_symlink()
    files = sorted(thumbnails.glob('*.jpg'))
    assert len(files) >= reuse.SOURCE_FILES and all(path.is_file() and not path.is_symlink() for path in files)
    return sha(database), {path.name: sha(path) for path in files}


def lifecycle(sealed):
    database = b.DATA / 'catalog.sqlite'
    control = b.WORK / 'control'
    control.mkdir()
    b.STATE.update(status='ready', application=str(b.APP_PATH))
    b.event('readyForNativeGui', application=str(b.APP_PATH), source=str(b.WORK / 'fixture/source'),
            controlDirectory=str(control), **b.check())
    steps = iter(STEPS)
    current = next(steps)
    until = time.monotonic() + 900
    while time.monotonic() < until:
        b.check()
        for path in sorted(control.glob('*.json')):
            label = json.loads(path.read_text())['op']
            path.unlink()
            assert label == current, f'Expected {current}, received {label}'
            assert reuse.source_state() == b.STATE['sourceBefore'] and reuse.normal_state() == b.STATE['normalBefore']
            if label in ('before-import', 'before-remove', 'after-reinstall'):
                pid = picker.verified_process(database)
                fingerprint = read_catalog(database, empty=label == 'before-import')
                if label == 'before-import':
                    b.STATE['originalPid'] = pid
                elif label == 'before-remove':
                    assert pid == b.STATE['originalPid']
                    b.STATE['catalogIdentitySha256'] = fingerprint
                else:
                    assert pid != b.STATE['originalPid'] and fingerprint == b.STATE['catalogIdentitySha256']
                b.event('isolatedCatalogVerified', label=label, pid=pid, media=0 if fingerprint is None else 5)
            elif label == 'remove':
                ensure_closed()
                assert b.APP_PATH.parent == b.WORK / 'Applications' and not b.APP_PATH.is_symlink()
                assert read_catalog(database) == b.STATE['catalogIdentitySha256']
                before = persistent_files(database)
                shutil.rmtree(b.APP_PATH)
                assert not b.APP_PATH.exists() and persistent_files(database) == before
                assert reuse.source_state() == b.STATE['sourceBefore']
                b.event('appRemoved', catalogFilePreserved=True, cacheFilesPreserved=len(before[1]), sourcesPreserved=5)
            elif label == 'reinstall':
                ensure_closed()
                install(sealed)
                b.event('appReinstalled', sameSignedPrograms=True)
            elif label == 'finish':
                ensure_closed()
                b.STATE['status'] = 'finished'
                return
            current = next(steps)
        time.sleep(.25)
    raise TimeoutError('The isolated install lifecycle did not finish')


def main():
    assert not any(path.exists() or path.is_symlink() for path in [*reuse.OWNED, *b.EXTRA])
    subprocess.run([sys.executable, 'scripts/check-test-artifact-budget.py', '--planned-source-files', '5',
                    '--planned-files', '300', '--planned-bytes', str(768 * 1024**2)], cwd=reuse.ROOT, check=True)
    b.STATE['normalBefore'] = reuse.normal_state()
    info = json.loads((reuse.ROOT / 'artifacts/build-info.json').read_text())
    archive = reuse.ROOT / 'artifacts' / info['file']
    assert sha(archive) == info['sha256']
    b.STATE['acceptedRelease'] = dict(zipSha256=info['sha256'], executableSha256=info['executableSha256'])
    b.TEMP = tempfile.TemporaryDirectory(prefix='travel-gallery-native-install-')
    b.WORK = Path(b.TEMP.name).resolve()
    b.STATE['temporaryRoot'] = str(b.WORK)
    try:
        extracted = b.WORK / 'extracted'
        extracted.mkdir()
        b.command('extractAcceptedZip', ['/usr/bin/ditto', '-x', '-k', str(archive), str(extracted)])
        app = extracted / 'Travel Gallery.app'
        plist = app / 'Contents/Info.plist'
        metadata = plistlib.loads(plist.read_bytes())
        assert metadata['CFBundleIdentifier'] == 'com.travelgallery.desktop'
        subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)], check=True)
        assert programs(app) == {name: info[field] for name, field in PROGRAMS.items()}
        b.event('archiveAppVerified', fiveProgramDigestsMatched=True)
        source = b.WORK / 'fixture/source'
        source.mkdir(parents=True)
        reuse.write_sources(source)
        b.STATE['sourceBefore'] = reuse.source_state()
        marker = dict(identifier=b.IDENTIFIER, count=reuse.SOURCE_FILES, fixtureRoot=str(source))
        (source.parent / 'OWNED_NATIVE_BENCHMARK.json').write_text(json.dumps(marker))
        metadata.update(CFBundleIdentifier=b.IDENTIFIER, CFBundleName=Path(NAME).stem,
                        CFBundleDisplayName=Path(NAME).stem)
        plist.write_bytes(plistlib.dumps(metadata))
        qa = b.WORK / 'qa'
        b.command('sealIsolatedApp', [sys.executable, 'scripts/prepare-offline-sandbox.py', '--source', str(app),
                  '--fixture', str(source.parent), '--output', str(qa), '--webkit-mach-lookup'])
        shutil.rmtree(extracted)
        applications = b.WORK / 'Applications'
        applications.mkdir()
        b.APP_PATH = applications / NAME
        install(qa / NAME)
        b.OWNED_PATHS.extend(reuse.OWNED)
        lifecycle(qa / NAME)
    finally:
        b.STATE['normalPreserved'] = reuse.normal_state() == b.STATE['normalBefore']
        assert b.STATE['normalPreserved']


if __name__ == '__main__':
    if b.APPROVAL.exists():
        assert json.loads(b.APPROVAL.read_text())['status'] == 'closed'
    b.APPROVAL.write_text(json.dumps(dict(status='active', authorization='AGENTS.md default budget',
        scope='Isolated installation, App bundle removal and reinstall from the current ZIP',
        limits=dict(files=2000, logicalBytes=2 * 1024**3, sourceFiles=5)), indent=2) + '\n')
    b.main = main
    raise SystemExit(b.run())
