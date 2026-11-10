// Local review artifact only. Developer ID signing/notarization is a separate release gate.
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';

if (process.platform !== 'darwin') throw new Error('This packaging command requires macOS.');
if (process.env.APPLE_SIGNING_IDENTITY && process.env.APPLE_SIGNING_IDENTITY !== '-')
  throw new Error('This local-review command must not replace a Developer ID signature.');
const root = dirname(dirname(fileURLToPath(import.meta.url)));
const bundle = process.env.TRAVEL_GALLERY_PACKAGE_APP ?? join(root, 'src-tauri/target/release/bundle/macos/Travel Gallery.app');
if (!isAbsolute(bundle) || basename(bundle) !== 'Travel Gallery.app')
  throw new Error('The review bundle must be an absolute path named Travel Gallery.app.');
if (!existsSync(bundle)) throw new Error('Build the release .app first with pnpm desktop:build.');
const workers = ['travel-gallery-ai', 'travel-gallery-quality', 'ffmpeg', 'ffprobe'].map((name) =>
  join('Contents', 'MacOS', name),
);
for (const worker of workers) {
  if (!existsSync(join(bundle, worker)))
    throw new Error('A bundled native helper is missing. Rebuild the complete application.');
}
const mediaResources = join(bundle, 'Contents/Resources/media-tools');
const mediaBuild = JSON.parse(readFileSync(join(mediaResources, 'build-info.json'), 'utf8'));
const mediaProfile = JSON.parse(readFileSync(join(root, 'third-party/media-tools/profile.json'), 'utf8'));
const sourceHash = createHash('sha256').update(readFileSync(join(mediaResources, mediaProfile.archive))).digest('hex');
if (sourceHash !== mediaProfile.sha256 || mediaBuild.sourceSha256 !== sourceHash)
  throw new Error('Bundled media-tool corresponding source differs from the pinned release.');
const signature = spawnSync('codesign', ['-dv', '--verbose=2', bundle], { encoding: 'utf8' });
const team = signature.stderr?.match(/^TeamIdentifier=(.+)$/m)?.[1]?.trim();
if (team && team !== 'not set')
  throw new Error('This local-review command must not replace an existing team signature.');
const output = join(root, 'artifacts');
mkdirSync(output, { recursive: true });
const filename = `Travel-Gallery-0.1.0-macOS-${process.arch}.zip`;
const temporary = join(output, `.${randomUUID()}.zip`);
const extracted = mkdtempSync(join(tmpdir(), 'travel-gallery-package-check-'));
try {
  for (const worker of workers)
    execFileSync('codesign', ['--force', '--sign', '-', '--timestamp=none', join(bundle, worker)], {
      stdio: 'pipe',
    });
  execFileSync('codesign', ['--force', '--sign', '-', '--timestamp=none', bundle], { stdio: 'pipe' });
  for (const worker of workers)
    execFileSync('codesign', ['--verify', '--strict', join(bundle, worker)], { stdio: 'pipe' });
  execFileSync('codesign', ['--verify', '--deep', '--strict', bundle], { stdio: 'pipe' });
  execFileSync('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', bundle, temporary], {
    stdio: 'pipe',
  });
  execFileSync('/usr/bin/unzip', ['-t', temporary], { stdio: 'pipe' });
  execFileSync('ditto', ['-x', '-k', temporary, extracted], { stdio: 'pipe' });
  const extractedBundle = join(extracted, 'Travel Gallery.app');
  execFileSync('codesign', ['--verify', '--deep', '--strict', extractedBundle], { stdio: 'pipe' });
  const executable = join('Contents', 'MacOS', 'travel-gallery');
  const executableHash = createHash('sha256')
    .update(readFileSync(join(bundle, executable)))
    .digest('hex');
  if (
    createHash('sha256')
      .update(readFileSync(join(extractedBundle, executable)))
      .digest('hex') !== executableHash
  )
    throw new Error('Extracted executable differs from the packaged application.');
  const workerHashes = {};
  for (const worker of workers) {
    execFileSync('codesign', ['--verify', '--strict', join(extractedBundle, worker)], { stdio: 'pipe' });
    const hash = createHash('sha256')
      .update(readFileSync(join(bundle, worker)))
      .digest('hex');
    if (
      createHash('sha256')
        .update(readFileSync(join(extractedBundle, worker)))
        .digest('hex') !== hash
    )
      throw new Error('Extracted CPU worker differs from packaged application.');
    workerHashes[worker.split('/').at(-1)] = hash;
  }
  const hash = createHash('sha256').update(readFileSync(temporary)).digest('hex');
  renameSync(temporary, join(output, filename));
  writeFileSync(join(output, 'SHA256SUMS.txt'), `${hash}  ${filename}\n`);
  writeFileSync(
    join(output, 'build-info.json'),
    JSON.stringify(
      {
        createdAt: new Date().toISOString(),
        version: '0.1.0',
        platform: 'macOS',
        architecture: process.arch,
        file: filename,
        bytes: statSync(join(output, filename)).size,
        sha256: hash,
        signing: 'ad hoc local integrity seal; no Developer ID; not notarized',
        signatureVerification: 'codesign --verify --deep --strict passed',
        archiveVerification: 'unzip -t passed',
        extractedBundleVerification:
          'ditto extraction, codesign --verify --deep --strict, main executable and all four native helpers SHA-256 matches passed',
        executableSha256: executableHash,
        aiWorkerSha256: workerHashes['travel-gallery-ai'],
        qualityWorkerSha256: workerHashes['travel-gallery-quality'],
        ffmpegSha256: workerHashes.ffmpeg,
        ffprobeSha256: workerHashes.ffprobe,
        mediaToolsVersion: mediaBuild.version,
        mediaToolsSourceSha256: sourceHash,
        correspondingSourceBundled: true,
        bundledModelWeights: false,
        intendedUse:
          'Local review and testing; full PRD acceptance/release gates remain in docs/COMPLETION-AUDIT.md',
      },
      null,
      2,
    ) + '\n',
  );
  console.log(`${filename}: integrity signature and archive verified; SHA-256 ${hash}`);
} finally {
  rmSync(temporary, { force: true });
  rmSync(extracted, { recursive: true, force: true });
}
