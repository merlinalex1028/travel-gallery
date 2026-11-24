"""Verify existing notice materials locally, without Cargo downloads or regeneration."""
from pathlib import Path
import argparse
import copy
import datetime
import hashlib
import json
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
JS_WHITESPACE = '\t\n\v\f\r \u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000\ufeff'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def body_id(text):
    # ECMAScript trim also removes the byte-order mark.
    return digest(text.strip(JS_WHITESPACE).encode())[:16]


def verify(inventory, notice, supplements, cargo_versions, npm_versions, supplemental_bytes):
    components = inventory['components']
    records = {f"{p['ecosystem']}:{p['name']}@{p['version']}": p for p in components}
    require(len(records) == len(components), 'Duplicate component version')
    for ecosystem, expected in (('cargo', cargo_versions), ('npm', npm_versions)):
        actual = {(p['name'], p['version']) for p in components if p['ecosystem'] == ecosystem}
        require(actual == expected, f'{ecosystem} inventory differs from current versions: '
                f'missing={sorted(expected-actual)}, extra={sorted(actual-expected)}')
    table = notice.split('\n\nSUPPLEMENTAL NOTICE SOURCES\n', 1)[0]
    lines = [line for line in table.splitlines() if line.startswith(('cargo:', 'npm:', 'native:'))]
    expected_lines = [f"{key} | {p['license']} | {', '.join(p['noticeTextIds'])}" for key, p in records.items()]
    require(len(lines) == len(expected_lines) and set(lines) == set(expected_lines), 'Public notice component table differs from inventory')
    pieces = re.split(r'\n\n================ NOTICE ([0-9a-f]{16}) ================\n', notice)
    bodies = {pieces[i]: pieces[i+1] for i in range(1, len(pieces), 2)}
    require(len(bodies) == (len(pieces)-1)//2, 'Duplicate public notice body')
    used = {key for p in components for key in p['noticeTextIds']}
    require(all(p['noticeTextIds'] for p in components), 'A component has no notice text')
    require(used == bodies.keys(), 'Public notice body references are incomplete or orphaned')
    for key, text in bodies.items():
        require(body_id(text) == key, f'Public notice body checksum mismatch: {key}')
    review = {key for key, p in records.items() if any(
        p.get(field, {}).get('status') == 'needs-review' for field in ('supplementalReview', 'nativeRuntimeReview'))}
    require(review == set(inventory['packagesRequiringSupplementalReview']), 'Release review queue differs from component findings')
    require(not inventory['packagesWithoutAnyNoticeText'], 'Inventory records missing notice text')
    seen = set()
    for supplement in supplements['packages']:
        key = f"{supplement['ecosystem']}:{supplement['name']}@{supplement['version']}"
        require(key not in seen, f'Duplicate supplemental package: {key}')
        seen.add(key)
        require(key in records, f'Supplemental package absent from inventory: {key}')
        record = records[key]
        require(record.get('supplementalReview') == supplement['review'], f'Supplemental review differs: {key}')
        require(record.get('versionEvidence') == supplement['versionEvidence'], f'Version evidence differs: {key}')
        for item in supplement['files']:
            data = supplemental_bytes[item['file']]
            require(digest(data) == item['sha256'], f'Supplemental checksum mismatch: {item["file"]}')
            identifier = body_id(data.decode())
            require(identifier in record['noticeTextIds'], f'Supplemental body not bound to package: {key}')
            require(any(source.get('kind') == 'upstream' and source.get('noticeTextId') == identifier and
                all(source.get(field) == value for field, value in item.items())
                for source in record['noticeSources']), f'Supplemental provenance differs: {key}')
            require(f"{key} | {identifier} | {item['sourceUrl']} | SHA-256 {item['sha256']}" in notice,
                    f'Public supplemental provenance missing: {key}')
    return dict(componentVersions=len(records), noticeBodies=len(bodies), supplementalPackages=len(seen),
                supplementalFiles=len(supplemental_bytes), pendingReleaseReviews=len(review))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true', help='Reject known bad in-memory inputs too')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    paths = ['docs/dependency-licenses.json', 'public/third-party-notices.txt',
             'third-party/supplemental-notices.json', 'src-tauri/Cargo.lock', 'pnpm-lock.yaml']
    inputs = {name: (ROOT/name).read_bytes() for name in paths}
    inventory = json.loads(inputs[paths[0]])
    notice = inputs[paths[1]].decode()
    supplements = json.loads(inputs[paths[2]])
    cargo_versions = {(p['name'], p['version']) for p in tomllib.loads(inputs[paths[3]].decode())['package']
                      if p['name'] != 'travel-gallery'}
    result = subprocess.run(['pnpm', 'licenses', 'list', '--json'], cwd=ROOT, capture_output=True, text=True, check=True)
    npm_versions = set()
    for group in json.loads(result.stdout).values():
        for package in group:
            versions = set()
            for folder in package['paths']:
                actual = json.loads((Path(folder)/'package.json').read_bytes())
                require(actual['name'] == package['name'], f'Installed package name differs: {folder}')
                versions.add(actual['version'])
                npm_versions.add((actual['name'], actual['version']))
            require(versions == set(package['versions']), f'Installed package versions differ: {package["name"]}')
    supplemental_bytes = {}
    license_root = (ROOT/'third-party/licenses').resolve()
    for package in supplements['packages']:
        for item in package['files']:
            path = (ROOT/item['file']).resolve(strict=True)
            require(path.is_relative_to(license_root), f'Supplemental path escapes license folder: {item["file"]}')
            supplemental_bytes[item['file']] = path.read_bytes()
    summary = verify(inventory, notice, supplements, cargo_versions, npm_versions, supplemental_bytes)
    map_check = subprocess.run(['node', 'scripts/map-resources.mjs', '--check'], cwd=ROOT,
                               capture_output=True, text=True, check=True)
    map_resources = json.loads(map_check.stdout)
    ui_font_check = subprocess.run(['node', 'scripts/ui-font.mjs'], cwd=ROOT,
                                   capture_output=True, text=True, check=True)
    negative = []
    if args.self_test:
        missing = copy.deepcopy(inventory)
        missing['components'].pop(next(i for i, p in enumerate(missing['components']) if p['ecosystem'] == 'cargo'))
        missing_npm = copy.deepcopy(inventory)
        missing_npm['components'].pop(next(i for i, p in enumerate(missing_npm['components']) if p['ecosystem'] == 'npm'))
        stale_review = copy.deepcopy(inventory)
        stale_review['packagesRequiringSupplementalReview'] = []
        first_id = inventory['components'][0]['noticeTextIds'][0]
        changed = notice.replace(f'NOTICE {first_id} ================\n', f'NOTICE {first_id} ================\nChanged text\n', 1)
        modified_bytes = dict(supplemental_bytes)
        modified_bytes[next(iter(modified_bytes))] += b'changed'
        for label, data, text, files in (
            ('missing-cargo-version', missing, notice, supplemental_bytes),
            ('missing-npm-version', missing_npm, notice, supplemental_bytes),
            ('changed-body', inventory, changed, supplemental_bytes),
            ('missing-public-table-row', inventory, notice.replace('cargo:', 'removed:', 1), supplemental_bytes),
            ('erased-review-queue', stale_review, notice, supplemental_bytes),
            ('changed-supplemental-file', inventory, notice, modified_bytes),
        ):
            try:
                verify(data, text, supplements, cargo_versions, npm_versions, files)
            except ValueError as error:
                negative.append(dict(case=label, rejected=True, reason=str(error)[:300]))
            else:
                raise ValueError(f'Negative check accepted invalid materials: {label}')
    report = dict(atUtc=datetime.datetime.now(datetime.timezone.utc).isoformat(), passed=True, **summary,
                  offlineMapResources=map_resources,
                  offlineUiFont=json.loads(ui_font_check.stdout),
                  cargoVersions=len(cargo_versions), installedNpmVersions=len(npm_versions), negativeChecks=negative,
                  inputSha256={name: digest(data) for name, data in inputs.items()},
                  checkerSourceSha256=digest(Path(__file__).read_bytes()),
                  supplementalHashesVerified=True, dependencyDownloads=0, temporaryFilesCreated=0,
                  scope='Current local inventories, locked Cargo versions, installed npm versions, notice bodies and cached supplemental provenance. Includes development tools and platform alternatives; not the exact runtime link set or legal release clearance.')
    if args.report:
        path = args.report.absolute()
        require(path.parent == ROOT/'docs' and not path.is_symlink(), 'Report must be a regular path in docs')
        path.write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
    print(json.dumps(report, ensure_ascii=False))


if __name__ == '__main__':
    main()
