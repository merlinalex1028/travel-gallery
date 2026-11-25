import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  appendFileSync,
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
export const platforms = {
  'macos-arm64': { target: 'aarch64-apple-darwin', folder: 'dmg', extension: '.dmg' },
  'windows-x64': { target: 'x86_64-pc-windows-msvc', folder: 'nsis', extension: '.exe' },
};

export function releaseVersion(tag) {
  const match = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(
    tag ?? '',
  );
  if (
    !match ||
    match.slice(1, 4).some((part) => Number(part) > 65535) ||
    match[4]?.split('.').some((part) => /^\d+$/.test(part) && /^0\d/.test(part))
  ) {
    throw new Error(
      'Release tag must be vMAJOR.MINOR.PATCH, optionally with a SemVer prerelease (e.g. v1.2.3-beta.1); components must fit Windows version fields.',
    );
  }
  return tag.slice(1);
}

export function assetName(tag, platform) {
  const version = releaseVersion(tag);
  if (platform === 'macos-arm64') return `Travel-Gallery-${version}-macOS-arm64.dmg`;
  if (platform === 'windows-x64') return `Travel-Gallery-${version}-Windows-x64-setup.exe`;
  throw new Error(`Unsupported release platform: ${platform}`);
}

function run(command, args, cwd = root) {
  return execFileSync(command, args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
}

function requireEnvironment(names) {
  const missing = names.filter((name) => !process.env[name]?.trim());
  if (missing.length)
    throw new Error(`Missing release configuration: ${missing.join(', ')}. See docs/RELEASING.md.`);
}

export function releaseSigningMode(value = process.env.RELEASE_SIGNING ?? 'unsigned') {
  if (!['signed', 'unsigned'].includes(value)) throw new Error('RELEASE_SIGNING must be signed or unsigned.');
  return value;
}

export function signingDescription(platform, mode = releaseSigningMode()) {
  if (!platforms[platform]) throw new Error(`Unsupported release platform: ${platform}`);
  releaseSigningMode(mode);
  if (mode === 'unsigned') return platform === 'macos-arm64' ? 'ad-hoc; not notarized' : 'unsigned';
  return platform === 'macos-arm64' ? 'Developer ID; notarized and stapled' : 'Authenticode; timestamped';
}

export function synchronizeVersion(directory, tag) {
  const version = releaseVersion(tag);
  const updates = new Map();
  for (const name of ['package.json', 'src-tauri/tauri.conf.json']) {
    const value = JSON.parse(readFileSync(path.join(directory, name), 'utf8'));
    value.version = version;
    updates.set(name, `${JSON.stringify(value, null, 2)}\n`);
  }
  for (const [name, pattern] of [
    ['src-tauri/Cargo.toml', /(\[package\]\s*\nname = "travel-gallery"\s*\nversion = ")[^"]+(")/g],
    ['src-tauri/Cargo.lock', /(\[\[package\]\]\s*\nname = "travel-gallery"\s*\nversion = ")[^"]+(")/g],
  ]) {
    const text = readFileSync(path.join(directory, name), 'utf8');
    if ([...text.matchAll(pattern)].length !== 1)
      throw new Error(`Cannot uniquely locate travel-gallery version in ${name}`);
    updates.set(
      name,
      text.replace(pattern, (_match, before, after) => `${before}${version}${after}`),
    );
  }
  // Validate every file before changing any of them. Changes stay in the disposable runner checkout.
  for (const [name, text] of updates) writeFileSync(path.join(directory, name), text);
}

export function signingConfig(directory, platform, mode = releaseSigningMode()) {
  releaseSigningMode(mode);
  if (mode === 'unsigned') {
    if (platform === 'macos-arm64')
      return { bundle: { macOS: { signingIdentity: '-', hardenedRuntime: false } } };
    if (platform === 'windows-x64')
      return { bundle: { windows: { signCommand: null, certificateThumbprint: null } } };
    throw new Error(`Unsupported release platform: ${platform}`);
  }
  if (platform === 'macos-arm64') return { bundle: { macOS: { hardenedRuntime: true } } };
  if (platform !== 'windows-x64') throw new Error(`Unsupported release platform: ${platform}`);
  return {
    bundle: {
      windows: {
        signCommand: {
          cmd: 'pwsh',
          args: [
            '-NoProfile',
            '-NonInteractive',
            '-File',
            path.join(directory, 'scripts/sign-windows.ps1'),
            '-FilePath',
            '%1',
          ],
        },
      },
    },
  };
}

function validateCommit(commit) {
  if (!/^[0-9a-f]{40}$/.test(commit ?? ''))
    throw new Error('Expected a resolved 40-character release commit SHA.');
}

function sha256(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex');
}

export function collectArtifact(directory, tag, commit, platform) {
  validateCommit(commit);
  const filename = assetName(tag, platform);
  const spec = platforms[platform];
  const bundle = path.join(directory, 'src-tauri/target', spec.target, 'release/bundle', spec.folder);
  const candidates = readdirSync(bundle).filter((name) => name.endsWith(spec.extension));
  if (candidates.length !== 1)
    throw new Error(`Expected one ${platform} installer, found ${candidates.length}`);
  const source = path.join(bundle, candidates[0]);
  if (!statSync(source).isFile() || statSync(source).size === 0)
    throw new Error('Installer is missing or empty.');
  const output = path.join(directory, 'release-dist');
  mkdirSync(output, { recursive: true });
  const destination = path.join(output, filename);
  copyFileSync(source, destination);
  const receipt = {
    tag,
    version: releaseVersion(tag),
    commit,
    platform,
    target: spec.target,
    signing: signingDescription(platform),
    file: filename,
    bytes: statSync(destination).size,
    sha256: sha256(destination),
  };
  writeFileSync(path.join(output, `${platform}.json`), `${JSON.stringify(receipt, null, 2)}\n`);
  return receipt;
}

export function verifyReleaseArtifacts(directory, tag, commit) {
  validateCommit(commit);
  const files = [];
  const sums = [];
  for (const platform of Object.keys(platforms)) {
    const file = assetName(tag, platform);
    const receiptName = `${platform}.json`;
    const receipt = JSON.parse(readFileSync(path.join(directory, receiptName), 'utf8'));
    const installer = path.join(directory, file);
    if (
      receipt.tag !== tag ||
      receipt.version !== releaseVersion(tag) ||
      receipt.commit !== commit ||
      receipt.platform !== platform ||
      receipt.target !== platforms[platform].target ||
      receipt.signing !== signingDescription(platform) ||
      receipt.file !== file ||
      receipt.bytes !== statSync(installer).size ||
      receipt.sha256 !== sha256(installer)
    ) {
      throw new Error(`Release artifact provenance/checksum mismatch: ${platform}`);
    }
    sums.push(`${receipt.sha256}  ${file}`);
    sums.push(`${sha256(path.join(directory, receiptName))}  ${receiptName}`);
    files.push(installer, path.join(directory, receiptName));
  }
  const checksum = path.join(directory, 'SHA256SUMS.txt');
  writeFileSync(checksum, `${sums.join('\n')}\n`);
  return [...files, checksum];
}

async function existingRelease(tag) {
  requireEnvironment(['GH_TOKEN', 'GITHUB_REPOSITORY']);
  const response = await fetch(
    `https://api.github.com/repos/${process.env.GITHUB_REPOSITORY}/releases/tags/${encodeURIComponent(tag)}`,
    {
      headers: {
        Authorization: `Bearer ${process.env.GH_TOKEN}`,
        Accept: 'application/vnd.github+json',
        'X-GitHub-Api-Version': '2022-11-28',
      },
    },
  );
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Cannot inspect GitHub Release: HTTP ${response.status}`);
  const release = await response.json();
  if (release.immutable)
    throw new Error(
      'This published Release is immutable. Push a new version tag so assets can be attached to a draft before publication. See docs/RELEASING.md.',
    );
  return release;
}

async function main() {
  const [command, platform] = process.argv.slice(2);
  const tag = process.env.RELEASE_TAG;
  const version = releaseVersion(tag);
  if (command === 'resolve') {
    const commit = run('git', ['rev-parse', '--verify', `refs/tags/${tag}^{commit}`]);
    validateCommit(commit);
    if (process.env.GITHUB_EVENT_NAME === 'push' && process.env.GITHUB_SHA !== commit) {
      throw new Error('Release tag moved after the push event.');
    }
    await existingRelease(tag);
    appendFileSync(process.env.GITHUB_OUTPUT, `tag=${tag}\nversion=${version}\ncommit=${commit}\n`);
  } else if (command === 'prepare') {
    // Never rewrite local release metadata or use a developer's signing identity by accident.
    if (process.env.GITHUB_ACTIONS !== 'true')
      throw new Error('Release preparation is only allowed in a disposable GitHub Actions checkout.');
    const expected = platform === 'macos-arm64' ? ['darwin', 'arm64'] : ['win32', 'x64'];
    if (!platforms[platform] || process.platform !== expected[0] || process.arch !== expected[1]) {
      throw new Error(`Wrong native release runner for ${platform}: ${process.platform}/${process.arch}`);
    }
    const mode = releaseSigningMode();
    if (mode === 'signed' && platform === 'macos-arm64') {
      requireEnvironment([
        'APPLE_CERTIFICATE',
        'APPLE_CERTIFICATE_PASSWORD',
        'APPLE_SIGNING_IDENTITY',
        'APPLE_API_KEY',
        'APPLE_API_ISSUER',
        'APPLE_API_PRIVATE_KEY',
      ]);
      if (!/^Developer ID Application: .+ \([A-Z0-9]{10}\)$/.test(process.env.APPLE_SIGNING_IDENTITY)) {
        throw new Error(
          'APPLE_SIGNING_IDENTITY must be a Developer ID Application identity, including its Team ID.',
        );
      }
      const key = path.join(process.env.RUNNER_TEMP, 'travel-gallery-notary.p8');
      writeFileSync(key, process.env.APPLE_API_PRIVATE_KEY, { mode: 0o600 });
      appendFileSync(process.env.GITHUB_ENV, `APPLE_API_KEY_PATH=${key}\n`);
    } else if (mode === 'signed') {
      requireEnvironment([
        'AZURE_CLIENT_ID',
        'AZURE_TENANT_ID',
        'AZURE_SUBSCRIPTION_ID',
        'ARTIFACT_SIGNING_ENDPOINT',
        'ARTIFACT_SIGNING_ACCOUNT',
        'ARTIFACT_SIGNING_PROFILE',
        'WINDOWS_PUBLISHER_NAME',
      ]);
      if (
        !/^https:\/\/[a-z0-9.-]+\.codesigning\.azure\.net\/?$/.test(process.env.ARTIFACT_SIGNING_ENDPOINT)
      ) {
        throw new Error(
          'ARTIFACT_SIGNING_ENDPOINT must be the HTTPS endpoint from your Azure Artifact Signing account.',
        );
      }
    }
    console.log(`Release mode: ${signingDescription(platform, mode)}`);
    synchronizeVersion(root, tag);
    writeFileSync(
      path.join(process.env.RUNNER_TEMP, 'travel-gallery-signing.json'),
      `${JSON.stringify(signingConfig(root, platform), null, 2)}\n`,
    );
  } else if (command === 'collect') {
    console.log(JSON.stringify(collectArtifact(root, tag, process.env.RELEASE_COMMIT, platform), null, 2));
  } else if (command === 'publish') {
    const commit = process.env.RELEASE_COMMIT;
    const files = verifyReleaseArtifacts(path.join(root, 'release-dist'), tag, commit);
    // Check the remote tag again after the potentially long native builds.
    requireEnvironment(['GH_TOKEN', 'GITHUB_REPOSITORY']);
    const current = run('gh', [
      'api',
      `repos/${process.env.GITHUB_REPOSITORY}/commits/${encodeURIComponent(`refs/tags/${tag}`)}`,
      '--jq',
      '.sha',
    ]);
    if (current !== commit)
      throw new Error('Remote release tag changed during the build; refusing to publish.');
    const release = await existingRelease(tag);
    const repo = ['--repo', process.env.GITHUB_REPOSITORY];
    if (release) {
      run('gh', ['release', 'upload', tag, ...files, '--clobber', ...repo]);
    } else {
      const notes = path.join(root, 'release-dist/RELEASE-NOTES.md');
      writeFileSync(
        notes,
        `Travel Gallery ${tag}\n\nmacOS arm64 DMG: ${signingDescription('macos-arm64')}.\nWindows x64 installer: ${signingDescription('windows-x64')}.\n\n${releaseSigningMode() === 'unsigned' ? 'These builds have no trusted publisher signature. macOS Gatekeeper or Windows SmartScreen may require manual approval. See the repository README for installation instructions.\n\n' : ''}Verify downloads with SHA256SUMS.txt.\nSource commit: ${commit}\n`,
      );
      run('gh', [
        'release',
        'create',
        tag,
        ...files,
        notes,
        '--draft',
        '--verify-tag',
        '--title',
        `Travel Gallery ${tag}`,
        '--notes-file',
        notes,
        ...(version.includes('-') ? ['--prerelease'] : []),
        ...repo,
      ]);
    }
    if (!release || release.draft) run('gh', ['release', 'edit', tag, '--draft=false', ...repo]);
    console.log(`Published both verified installers for ${tag} (${commit}).`);
  } else {
    throw new Error('Usage: node scripts/release.mjs resolve|prepare PLATFORM|collect PLATFORM|publish');
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    if (error.stderr) console.error(String(error.stderr));
    process.exitCode = 1;
  });
}
