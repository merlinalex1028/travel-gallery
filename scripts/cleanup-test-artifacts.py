"""Delete generated QA data; default is a preview. See docs/TEST-ARTIFACT-CLEANUP.md."""
from pathlib import Path
import argparse
import datetime
import hashlib
import json
import os
import plistlib
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
ARTIFACTS = ROOT / 'artifacts'
APP = ROOT / 'src-tauri/target/release/bundle/macos/Travel Gallery.app'
IDENTIFIERS = ('com.travelgallery.offlineaudit', 'com.travelgallery.benchmark', 'com.travelgallery.development')


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def available():
    stat = os.statvfs(ROOT)
    return stat.f_bavail * stat.f_frsize


def measure(path):
    files = jpg = logical = directories = 0
    if path.is_symlink() or not path.is_dir():
        entries = [path]
    else:
        entries = None
    def consume(entry):
        nonlocal files, jpg, logical
        stat = entry.lstat()
        files += 1
        logical += stat.st_size
        jpg += entry.suffix.lower() in ('.jpg', '.jpeg')
    if entries is not None:
        for entry in entries:
            consume(entry)
    else:
        for base, dirs, names in os.walk(path, followlinks=False):
            if os.path.ismount(base):
                raise RuntimeError(f'Refusing mounted directory: {base}')
            directories += 1
            for name in names:
                consume(Path(base) / name)
            for name in dirs:
                entry = Path(base) / name
                if entry.is_symlink():
                    consume(entry)
    return dict(files=files, jpgFiles=jpg, directories=directories, logicalBytes=logical)


def build_cache_targets():
    parent = ROOT / 'src-tauri/target'
    # Preserve exactly the accepted production App, removing sibling build outputs.
    for component in ('release', 'bundle', 'macos', 'Travel Gallery.app'):
        if not parent.is_dir():
            break
        for child in parent.iterdir():
            if child.name != component:
                yield child
        parent /= component


def qa_namespace_paths(home, identifier):
    if identifier not in IDENTIFIERS:
        raise ValueError('Expected an isolated application identifier')
    library = home / 'Library'
    container = library / 'Containers' / identifier / 'Data/Library'
    for folder, suffix in (('Application Support', ''), ('Caches', ''), ('WebKit', ''),
                           ('HTTPStorages', ''), ('HTTPStorages', '.binarycookies'),
                           ('Preferences', '.plist'), ('Saved Application State', '.savedState'),
                           ('Logs', '')):
        for path in (library / folder / (identifier + suffix),
                     container / folder / (identifier + suffix),
                     # A production identifier is disposable only inside a QA container.
                     container / folder / ('com.travelgallery.desktop' + suffix)):
            # Refuse linked ancestors even before a new profile exists.
            for parent in path.parents:
                if parent == home:
                    break
                if parent.is_symlink():
                    raise RuntimeError(f'Refusing linked QA ancestor: {parent}')
            yield path


def qa_data_targets(home=None):
    home = Path.home() if home is None else home
    for identifier in IDENTIFIERS:
        for path in qa_namespace_paths(home, identifier):
            if path.exists() or path.is_symlink():
                yield path


def remove_target(path):
    stats = measure(path)
    if path.is_symlink() or not path.is_dir():
        path.unlink()
    else:
        shutil.rmtree(path)
    assert not path.exists() and not path.is_symlink(), path
    return stats


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    parser.add_argument('--include-build-cache', action='store_true')
    parser.add_argument('--report', type=Path, default=ROOT / 'docs/test-artifact-cleanup-latest.json')
    args = parser.parse_args()
    assert json.loads((ROOT / 'package.json').read_text())['name'] == 'travel-gallery'
    assert not ARTIFACTS.is_symlink() and ARTIFACTS.resolve().parent == ROOT
    info = json.loads((ARTIFACTS / 'build-info.json').read_text())
    filename = info['file']
    assert Path(filename).name == filename and filename.endswith('.zip')
    kept = {filename, 'build-info.json', 'SHA256SUMS.txt'}
    assert sha(ARTIFACTS / filename) == info['sha256'], 'Current release ZIP does not match build-info'
    binaries = {'travel-gallery': 'executableSha256', 'travel-gallery-ai': 'aiWorkerSha256',
                'travel-gallery-quality': 'qualityWorkerSha256', 'ffmpeg': 'ffmpegSha256', 'ffprobe': 'ffprobeSha256'}
    if args.include_build_cache:
        for name, key in binaries.items():
            assert sha(APP / 'Contents/MacOS' / name) == info[key], 'Restore accepted App before build-cache cleanup'
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(APP)], check=True, capture_output=True)
    processes = subprocess.check_output(['ps', '-axo', 'pid=,comm='], text=True).splitlines()
    active = [line for line in processes if
              ('Travel Gallery' in line and '/Contents/MacOS/' in line) or str(ROOT / 'src-tauri/target/') in line]
    assert not active, f'Stop project app/test processes before deleting: {active}'
    images = plistlib.loads(subprocess.check_output(['hdiutil', 'info', '-plist']))
    assert not [i for i in images.get('images', []) if str(ARTIFACTS) in i.get('image-path', '')], 'Unmount owned test images first'
    targets = [p for p in ARTIFACTS.iterdir() if p.name not in kept]
    targets += [ROOT / name for name in ('dist-offline-canary', 'dist-performance', 'test-results') if (ROOT / name).exists()]
    targets += list(qa_data_targets())
    if args.include_build_cache:
        targets += list(build_cache_targets())
        generated = ROOT / 'src-tauri/gen'
        if generated.exists():
            targets.append(generated)
    normal = Path.home() / 'Library/Application Support/com.travelgallery.desktop'
    normal_before = {str(p): sha(p) for p in normal.rglob('*') if p.is_file()}
    source_roots = [ROOT / p for p in ('src', 'src-tauri/src', 'src-tauri/tests', 'tests')]
    sources_before = {str(p): sha(p) for base in source_roots for p in base.rglob('*') if p.is_file()}
    report = dict(startedAt=datetime.datetime.now(datetime.timezone.utc).isoformat(), applied=args.apply,
                  stage='planned', availableBefore=available(), targets=[str(p) for p in targets], removed=[],
                  keptRelease=sorted(kept), keptApp=str(APP), normalDataPath=str(normal),
                  scope='Generated project QA data/cache/build outputs only; no symlink traversal; source and normal catalog protected.')
    report_path = args.report.absolute()
    assert report_path.parent == ROOT / 'docs', 'Cleanup receipt must stay in docs, outside purge roots'
    def save():
        temporary = report_path.with_suffix('.tmp')
        temporary.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        temporary.replace(report_path)
    save()
    if not args.apply:
        print(json.dumps(dict(apply=False,targets=len(targets),report=str(report_path)), ensure_ascii=False))
        return
    report['stage'] = 'removing'
    save()
    for index, path in enumerate(targets, 1):
        assert path != normal and not normal.is_relative_to(path), 'Normal catalog cannot be a deletion target'
        if not path.exists() and not path.is_symlink():
            continue
        stats = remove_target(path)
        report['removed'].append(dict(path=str(path), **stats))
        save()
        print(json.dumps(dict(removed=index,total=len(targets),path=str(path.relative_to(ROOT)) if path.is_relative_to(ROOT) else str(path),**stats), ensure_ascii=False),flush=True)
    assert normal_before == {str(p): sha(p) for p in normal.rglob('*') if p.is_file()}, 'Normal data changed during cleanup'
    assert sources_before == {str(p): sha(p) for base in source_roots for p in base.rglob('*') if p.is_file()}, 'Product/test source changed during cleanup'
    assert {p.name for p in ARTIFACTS.iterdir()} == kept
    assert sha(ARTIFACTS / filename) == info['sha256']
    if args.include_build_cache:
        for name, key in binaries.items():
            assert sha(APP / 'Contents/MacOS' / name) == info[key]
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(APP)], check=True, capture_output=True)
    report.update(stage='complete', finishedAt=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  totals={key: sum(item[key] for item in report['removed']) for key in ('files','jpgFiles','directories','logicalBytes')},
                  availableAfter=available(), normalDataHashesUnchanged=True, sourceHashesUnchanged=True,
                  currentReleaseHashUnchanged=True, retainedArtifactEntries=sorted(kept))
    report['availableSpaceIncreaseBytes'] = report['availableAfter'] - report['availableBefore']
    save()
    print(json.dumps({k:report[k] for k in ('stage','totals','availableSpaceIncreaseBytes')}, ensure_ascii=False),flush=True)


if __name__ == '__main__':
    main()
