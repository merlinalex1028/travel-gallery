import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve, relative, isAbsolute } from 'node:path';
import { mapFontNotice } from './map-resources.mjs';
import { uiFontNotice } from './ui-font.mjs';
const runner = process.env.npm_execpath;
if (!runner) throw new Error('Run with pnpm notices.');
function json(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 });
  if (result.status !== 0)
    throw new Error(result.stderr || result.error?.message || 'Inventory command failed');
  return JSON.parse(result.stdout);
}
const npm = /\.(c?js|mjs)$/.test(runner)
  ? json(process.execPath, [runner, 'licenses', 'list', '--json'])
  : json(runner, ['licenses', 'list', '--json']);
const cargo = json(process.execPath, [
  'scripts/native.mjs',
  'cargo',
  'metadata',
  '--manifest-path',
  'src-tauri/Cargo.toml',
  '--features',
  'desktop,ai-worker,quality-worker',
  '--format-version',
  '1',
  '--locked',
]);
const texts = new Map(),
  records = new Map(),
  supplementalPath = 'third-party/supplemental-notices.json';
const supplemental = existsSync(supplementalPath)
  ? JSON.parse(readFileSync(supplementalPath, 'utf8')).packages
  : [];
const supplementalByKey = new Map();
for (const item of supplemental) {
  const key = `${item.ecosystem}:${item.name}@${item.version}`;
  if (supplementalByKey.has(key)) throw new Error(`Duplicate supplemental notice: ${key}`);
  supplementalByKey.set(key, item);
}
function add(ecosystem, name, version, license, folder) {
  const key = `${ecosystem}:${name}@${version}`;
  const record = records.get(key) ?? {
    ecosystem,
    name,
    version,
    license,
    noticeTextIds: [],
    noticeSources: [],
    hasRootNotice: false,
  };
  if (record.license !== license) throw new Error(`Conflicting license declarations for ${key}`);
  function textId(text) {
    const normalized = text.trim();
    if (!normalized) throw new Error(`Empty notice for ${key}`);
    const id = createHash('sha256').update(normalized).digest('hex').slice(0, 16);
    if (texts.has(id) && texts.get(id) !== normalized) throw new Error('Notice identifier collision');
    texts.set(id, normalized);
    if (!record.noticeTextIds.includes(id)) record.noticeTextIds.push(id);
    return id;
  }
  for (const file of readdirSync(folder)
    .filter((file) => /^(licen[sc]e|copying|notice|copyright)/i.test(file))
    .sort()) {
    const path = join(folder, file),
      info = statSync(path);
    if (!info.isFile() || info.size > 256 * 1024) continue;
    const text = readFileSync(path, 'utf8').trim();
    if (!text) continue;
    const id = textId(text);
    record.hasRootNotice = true;
    if (!record.noticeSources.some((source) => source.kind === 'packageFile' && source.file === file))
      record.noticeSources.push({ kind: 'packageFile', file, noticeTextId: id });
  }
  // Some publishers keep the complete terms in AUTHORS or a final README license section.
  if (!record.hasRootNotice) {
    for (const file of readdirSync(folder).filter((file) =>
      /^(authors(?:\..*)?|readme(?:\..*)?)$/i.test(file),
    )) {
      const path = join(folder, file);
      if (!statSync(path).isFile() || statSync(path).size > 256 * 1024) continue;
      const document = readFileSync(path, 'utf8');
      const heading = /^#{1,6}\s+Licen[sc]e[^\n]*\n/im.exec(document);
      let text = document;
      if (/^readme/i.test(file)) {
        if (!heading) continue;
        text = document.slice(heading.index);
        const next = /\n#{1,6}\s+[^\n]+\n/.exec(text);
        if (next) text = text.slice(0, next.index);
      }
      if (!/Permission is hereby granted[\s\S]*THE SOFTWARE IS PROVIDED[\s\S]*AS IS/i.test(text)) continue;
      const id = textId(text);
      if (!record.noticeSources.some((source) => source.kind === 'packageDocument' && source.file === file))
        record.noticeSources.push({
          kind: 'packageDocument',
          file,
          noticeTextId: id,
          section: heading ? heading[0].trim() : 'complete document',
        });
    }
  }
  const supplement = supplementalByKey.get(key);
  if (supplement && !record.noticeSources.some((source) => source.kind === 'upstream')) {
    for (const item of supplement.files) {
      const path = resolve(item.file);
      const within = relative(resolve('third-party/licenses'), path);
      if (within.startsWith('..') || isAbsolute(within))
        throw new Error(`Notice path escapes license folder: ${key}`);
      const physical = relative(realpathSync('third-party/licenses'), realpathSync(path));
      if (physical.startsWith('..') || isAbsolute(physical))
        throw new Error(`Notice symlink escapes license folder: ${key}`);
      const sourceUrl = new URL(item.sourceUrl);
      if (sourceUrl.protocol !== 'https:') throw new Error(`Notice source must use HTTPS: ${key}`);
      if (item.sourceKind === 'npmPackageFile') {
        const evidence = supplement.versionEvidence;
        if (
          ecosystem !== 'npm' ||
          sourceUrl.hostname !== 'registry.npmjs.org' ||
          evidence.name !== name ||
          evidence.version !== version ||
          item.packageIntegrity !== evidence.packageIntegrity ||
          !/^sha512-[A-Za-z0-9+/]+=*$/.test(item.packageIntegrity)
        )
          throw new Error(`Notice source must match the published npm tarball: ${key}`);
      } else if (!/^[a-f0-9]{40}$/.test(item.commit))
        throw new Error(`Notice source must use a fixed upstream commit: ${key}`);
      const bytes = readFileSync(path);
      if (createHash('sha256').update(bytes).digest('hex') !== item.sha256)
        throw new Error(`Supplemental notice checksum mismatch: ${key}`);
      if (item.sourceKind === 'npmPackageFile') {
        const declaration = JSON.parse(bytes.toString('utf8'));
        if (declaration.name !== name || declaration.version !== version || declaration.license !== license)
          throw new Error(`Published npm license declaration mismatch: ${key}`);
      }
      const id = textId(bytes.toString('utf8'));
      record.noticeSources.push({ kind: 'upstream', ...item, noticeTextId: id });
    }
    record.supplementalReview = supplement.review;
    record.versionEvidence = supplement.versionEvidence;
  }
  records.set(key, record);
}
for (const group of Object.values(npm))
  for (const pkg of group) {
    const seen = new Set();
    for (const folder of pkg.paths) {
      const actual = JSON.parse(readFileSync(join(folder, 'package.json'), 'utf8'));
      if (actual.name !== pkg.name || !pkg.versions.includes(actual.version))
        throw new Error(`Package metadata mismatch: ${folder}`);
      seen.add(actual.version);
      add(
        'npm',
        actual.name,
        actual.version,
        typeof actual.license === 'string' ? actual.license : pkg.license,
        folder,
      );
    }
    if (pkg.versions.some((version) => !seen.has(version)))
      throw new Error(`Missing installed package version: ${pkg.name}`);
  }
for (const pkg of cargo.packages)
  if (pkg.name !== 'travel-gallery')
    add(
      'cargo',
      pkg.name,
      pkg.version,
      pkg.license ?? pkg.license_file ?? 'UNSPECIFIED',
      dirname(pkg.manifest_path),
    );
const components = [...records.values()];
const mediaTools = JSON.parse(readFileSync('third-party/media-tools/profile.json', 'utf8'));
add('native', 'FFmpeg', mediaTools.version, 'LGPL-2.1-or-later', 'third-party/media-tools');
components.push(records.get(`native:FFmpeg@${mediaTools.version}`));
const builtMediaInfo = 'src-tauri/resources/media-tools/build-info.json';
if (existsSync(builtMediaInfo)) {
  const built = JSON.parse(readFileSync(builtMediaInfo, 'utf8'));
  if (built.target === 'x86_64-pc-windows-msvc') {
    if (!built.runtimePackages?.length) throw new Error('Windows media tools are missing their runtime notice inventory.');
    for (const pkg of built.runtimePackages) {
      const folder = resolve('src-tauri/resources/media-tools', pkg.noticeDirectory);
      const within = relative(resolve('src-tauri/resources/media-tools/windows-toolchain'), folder);
      if (within.startsWith('..') || isAbsolute(within)) throw new Error('Native runtime notice path escapes its package folder.');
      add('native', pkg.name, pkg.version, pkg.license, folder);
      const runtime = records.get(`native:${pkg.name}@${pkg.version}`);
      runtime.nativeRuntimeReview = { status: 'needs-review', reason: 'Installed MSYS2 runtime package declarations and complete notice texts collected; concrete static-link and commercial distribution review remains.' };
      components.push(runtime);
    }
  }
}
components.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
const missingRoot = components.filter((p) => !p.hasRootNotice).map((p) => `${p.name}@${p.version}`);
const missing = components
  .filter((p) => !p.noticeTextIds.length)
  .map((p) => `${p.ecosystem}:${p.name}@${p.version}`);
const inventory = {
  generatedAt: new Date().toISOString(),
  scope:
    'Installed npm package versions, Cargo metadata dependencies across platform alternatives and bundled native media tools; includes build tools',
  components,
  packagesWithoutRootNoticeFile: missingRoot,
  packagesWithoutAnyNoticeText: missing,
  packagesRequiringSupplementalReview: components
    .filter((p) => p.supplementalReview?.status === 'needs-review' || p.nativeRuntimeReview?.status === 'needs-review')
    .map((p) => `${p.ecosystem}:${p.name}@${p.version}`),
};
writeFileSync('third-party/dependency-inventory.json', JSON.stringify(inventory, null, 2) + '\n');
const header =
  'TRAVEL GALLERY — THIRD-PARTY NOTICES\n\nThis inventory includes development dependencies and platform alternatives, beyond the code distributed in any single build.\nFFmpeg/FFprobe 9.0.2 are separate bundled command-line programs under LGPL-2.1-or-later. Complete corresponding unmodified source, build configuration, verification record and license texts are in Contents/Resources/media-tools (macOS). FFmpeg is not owned by Travel Gallery. Test media are not bundled.\nMap geometry: Natural Earth, public domain. https://www.naturalearthdata.com/about/terms-of-use/\n\n';
const table = components
  .map(
    (p) =>
      `${p.ecosystem}:${p.name}@${p.version} | ${p.license} | ${p.noticeTextIds.join(', ') || 'see package license declaration'}`,
  )
  .join('\n');
const notices = [...texts]
  .map(([id, text]) => `\n\n================ NOTICE ${id} ================\n${text}`)
  .join('');
const provenance = components
  .flatMap((p) =>
    p.noticeSources
      .filter((s) => s.kind === 'upstream')
      .map(
        (s) =>
          `${p.ecosystem}:${p.name}@${p.version} | ${s.noticeTextId} | ${s.sourceUrl} | SHA-256 ${s.sha256}`,
      ),
  )
  .join('\n');
const sources = components
  .filter((p) => p.versionEvidence?.sourceArchive)
  .map((p) => `${p.ecosystem}:${p.name}@${p.version} | ${p.versionEvidence.sourceArchive}`)
  .join('\n');
writeFileSync(
  'public/third-party-notices.txt',
  header +
    mapFontNotice() +
    uiFontNotice() +
    table +
    '\n\nSUPPLEMENTAL NOTICE SOURCES\n' +
    provenance +
    '\n\nUNMODIFIED SOURCE ARCHIVES\n' +
    sources +
    notices +
    '\n',
);
console.log(
  `${components.length} package versions; ${texts.size} notice texts; ${missingRoot.length} lack root notice files; ${missing.length} still lack any notice text. Supplemental review findings are not a legal clearance.`,
);
