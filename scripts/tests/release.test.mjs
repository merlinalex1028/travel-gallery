import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import { createHash, generateKeyPairSync, sign } from 'node:crypto';
import { verifyUpdateSignature } from '../update-signature.mjs';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import {
  assetName,
  collectArtifact,
  platforms,
  releaseVersion,
  releaseSigningMode,
  signingDescription,
  signingConfig,
  synchronizeVersion,
  verifyReleaseArtifacts,
  updateAssetName,
} from '../release.mjs';

const commit = 'a'.repeat(40);
const tag = 'v1.2.3-beta.1';
const keys = generateKeyPairSync('ed25519');
const id = Buffer.alloc(8, 42);
const rawPublic = keys.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
const publicKey = Buffer.from(
  `untrusted comment: test key\n${Buffer.concat([Buffer.from('Ed'), id, rawPublic]).toString('base64')}\n`,
).toString('base64');
function signature(bytes, version = releaseVersion(tag), prehashed = true) {
  const sig = sign(
    null,
    prehashed ? createHash('blake2b512').update(bytes).digest() : bytes,
    keys.privateKey,
  );
  const trusted = `timestamp:1\tfile:fixture\tversion:${version}`;
  return Buffer.from(
    `untrusted comment: fixture\n${Buffer.concat([Buffer.from(prehashed ? 'ED' : 'Ed'), id, sig]).toString('base64')}\ntrusted comment: ${trusted}\n${sign(null, Buffer.concat([sig, Buffer.from(trusted)]), keys.privateKey).toString('base64')}\n`,
  ).toString('base64');
}

function fixture(t) {
  const directory = mkdtempSync(path.join(tmpdir(), 'travel-gallery-release-test-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  return directory;
}

function write(directory, name, value) {
  const file = path.join(directory, name);
  mkdirSync(path.dirname(file), { recursive: true });
  writeFileSync(file, value);
}

function installers(directory) {
  for (const [platform, spec] of Object.entries(platforms)) {
    write(
      directory,
      `src-tauri/target/${spec.target}/release/bundle/${spec.folder}/installer${spec.extension}`,
      `fixture:${platform}`,
    );
    const updatePath =
      platform === 'macos-arm64'
        ? `src-tauri/target/${spec.target}/release/bundle/macos/fixture.app.tar.gz`
        : `src-tauri/target/${spec.target}/release/bundle/${spec.folder}/installer${spec.extension}`;
    const bytes = Buffer.from(`fixture:${platform}`);
    write(directory, updatePath, bytes);
    write(directory, `${updatePath}.sig`, signature(bytes));
    collectArtifact(directory, tag, commit, platform, publicKey);
  }
  return path.join(directory, 'release-dist');
}

test('accepts supported release versions and rejects ambiguous or executable tag inputs', () => {
  for (const value of ['v0.1.0', tag, 'v2.0.0-rc.0', 'v65535.0.0']) {
    assert.equal(releaseVersion(value), value.slice(1));
  }
  for (const value of [
    undefined,
    '',
    'main',
    'v01.2.3',
    'v1.2',
    'v1.2.3+metadata',
    'v1.2.3-beta.01',
    'v65536.0.0',
    'v1.2.3\n',
    'v1.2.3;echo bad',
    'v1.2.3-$(whoami)',
    'v1.2.3-../../a',
  ])
    assert.throws(() => releaseVersion(value));
});

test('synchronizes all four versions and keeps unrelated lockfile entries intact', (t) => {
  const directory = fixture(t);
  write(directory, 'package.json', '{"name":"travel-gallery","version":"0.1.0"}\n');
  write(
    directory,
    'src-tauri/tauri.conf.json',
    '{"version":"0.1.0","identifier":"com.travelgallery.desktop"}\n',
  );
  const manifest = '[package]\nname = "travel-gallery"\nversion = "0.1.0"\nedition = "2021"\n';
  const other = '[[package]]\nname = "other"\nversion = "0.1.0"\n';
  write(directory, 'src-tauri/Cargo.toml', manifest);
  write(
    directory,
    'src-tauri/Cargo.lock',
    `version = 4\n\n${other}\n[[package]]\nname = "travel-gallery"\nversion = "0.1.0"\n`,
  );
  synchronizeVersion(directory, tag);
  for (const name of ['package.json', 'src-tauri/tauri.conf.json']) {
    assert.equal(JSON.parse(readFileSync(path.join(directory, name), 'utf8')).version, '1.2.3-beta.1');
  }
  assert.equal(
    readFileSync(path.join(directory, 'src-tauri/Cargo.toml'), 'utf8'),
    manifest.replace('0.1.0', '1.2.3-beta.1'),
  );
  const lock = readFileSync(path.join(directory, 'src-tauri/Cargo.lock'), 'utf8');
  assert.ok(lock.includes(other));
  assert.match(lock, /name = "travel-gallery"\nversion = "1\.2\.3-beta\.1"/);
});

test('invalid manifest layout fails before changing any version file', (t) => {
  const directory = fixture(t);
  const pkg = '{"name":"travel-gallery","version":"0.1.0"}\n';
  write(directory, 'package.json', pkg);
  write(directory, 'src-tauri/tauri.conf.json', '{"version":"0.1.0"}\n');
  write(directory, 'src-tauri/Cargo.toml', '[package]\nname = "unexpected"\nversion = "0.1.0"\n');
  write(directory, 'src-tauri/Cargo.lock', '');
  assert.throws(() => synchronizeVersion(directory, tag), /Cannot uniquely locate/);
  assert.equal(readFileSync(path.join(directory, 'package.json'), 'utf8'), pkg);
});

test('signing config preserves spaces and uses a literal Tauri file placeholder', () => {
  const directory = path.resolve('repository with spaces');
  const command = signingConfig(directory, 'windows-x64', 'signed').bundle.windows.signCommand;
  assert.equal(command.cmd, 'pwsh');
  assert.deepEqual(command.args, [
    '-NoProfile',
    '-NonInteractive',
    '-File',
    path.join(directory, 'scripts/sign-windows.ps1'),
    '-FilePath',
    '%1',
  ]);
  assert.equal(signingConfig(directory, 'macos-arm64', 'signed').bundle.macOS.hardenedRuntime, true);
  assert.throws(() => signingConfig(directory, 'windows-arm64'));
});

test('unsigned mode requires no external signing identity and labels each platform honestly', () => {
  const config = signingConfig('/repository', 'macos-arm64', 'unsigned');
  assert.equal(config.bundle.macOS.signingIdentity, '-');
  assert.equal(config.bundle.macOS.hardenedRuntime, false);
  assert.equal(signingConfig('/repository', 'windows-x64', 'unsigned').bundle.windows.signCommand, null);
  assert.equal(signingDescription('macos-arm64', 'unsigned'), 'ad-hoc; not notarized');
  assert.equal(signingDescription('windows-x64', 'unsigned'), 'unsigned');
  assert.throws(() => releaseSigningMode('false'));
});

test('requires both platform artifacts from the same tag/commit and checks file contents', (t) => {
  const directory = fixture(t);
  const output = installers(directory);
  assert.equal(verifyReleaseArtifacts(output, tag, commit, publicKey).length, 9);
  const sums = readFileSync(path.join(output, 'SHA256SUMS.txt'), 'utf8');
  assert.equal(sums.trim().split('\n').length, 8);
  assert.ok(sums.includes(assetName(tag, 'macos-arm64')));
  assert.throws(() => verifyReleaseArtifacts(output, tag, 'b'.repeat(40), publicKey), /provenance/);
  writeFileSync(path.join(output, assetName(tag, 'windows-x64')), 'tampered installer');
  assert.throws(() => verifyReleaseArtifacts(output, tag, commit, publicKey), /provenance/);
});

test('rejects a missing platform, mixed receipt target, ambiguous or empty installers', (t) => {
  const directory = fixture(t);
  const output = installers(directory);
  const receiptPath = path.join(output, 'windows-x64.json');
  const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
  receipt.target = 'aarch64-pc-windows-msvc';
  writeFileSync(receiptPath, JSON.stringify(receipt));
  assert.throws(() => verifyReleaseArtifacts(output, tag, commit, publicKey), /provenance/);
  rmSync(receiptPath);
  assert.throws(() => verifyReleaseArtifacts(output, tag, commit, publicKey), /ENOENT/);
  const spec = platforms['macos-arm64'];
  const folder = `src-tauri/target/${spec.target}/release/bundle/dmg/`;
  write(directory, `${folder}stale.dmg`, 'stale');
  assert.throws(() => collectArtifact(directory, tag, commit, 'macos-arm64'), /Expected one/);
  rmSync(path.join(directory, folder, 'stale.dmg'));
  write(directory, `${folder}installer.dmg`, '');
  assert.throws(() => collectArtifact(directory, tag, commit, 'macos-arm64'), /empty/);
});

test('manifest includes both official targets and rejects tampered or missing update artifacts', (t) => {
  const directory = fixture(t),
    output = installers(directory);
  verifyReleaseArtifacts(output, tag, commit, publicKey);
  const manifest = JSON.parse(readFileSync(path.join(output, 'latest.json'), 'utf8'));
  assert.equal(manifest.version, releaseVersion(tag));
  assert.deepEqual(Object.keys(manifest.platforms), ['darwin-aarch64', 'windows-x86_64']);
  for (const [target, entry] of Object.entries(manifest.platforms)) {
    const platform = target === 'darwin-aarch64' ? 'macos-arm64' : 'windows-x64';
    assert.equal(
      entry.url,
      `https://github.com/merlinalex1028/travel-gallery/releases/download/${tag}/${updateAssetName(tag, platform)}`,
    );
    verifyUpdateSignature(
      readFileSync(path.join(output, updateAssetName(tag, platform))),
      entry.signature,
      publicKey,
      manifest.version,
    );
  }
  const sigPath = path.join(output, `${updateAssetName(tag, 'macos-arm64')}.sig`);
  writeFileSync(sigPath, 'corrupted');
  assert.throws(() => verifyReleaseArtifacts(output, tag, commit, publicKey), /signature/);
  rmSync(sigPath);
  assert.throws(() => verifyReleaseArtifacts(output, tag, commit, publicKey), /ENOENT/);
});

test('update signatures bind the payload, public key, trusted comment and release version', () => {
  const bytes = Buffer.from('payload');
  for (const prehashed of [true, false]) {
    const sig = signature(bytes, '1.0.0', prehashed);
    verifyUpdateSignature(bytes, sig, publicKey, '1.0.0');
    assert.throws(() => verifyUpdateSignature(Buffer.from('changed'), sig, publicKey, '1.0.0'), /signature/);
    assert.throws(() => verifyUpdateSignature(bytes, sig, publicKey, '1.0.1'), /signature/);
    assert.throws(() => verifyUpdateSignature(bytes, sig, 'bad key', '1.0.0'), /signature/);
    const changed = Buffer.from(
      Buffer.from(sig, 'base64').toString().replace('version:1.0.0', 'version:1.0.1'),
    ).toString('base64');
    assert.throws(() => verifyUpdateSignature(bytes, changed, publicKey, '1.0.1'), /signature/);
  }
});

test('signature verifier accepts a version-bound signature produced by the installed Tauri CLI', (t) => {
  const directory = fixture(t),
    key = path.join(directory, 'test.key'),
    payload = path.join(directory, 'payload');
  // All key material stays in the temporary fixture and is removed by t.after.
  const cli = path.resolve('node_modules/@tauri-apps/cli/tauri.js');
  execFileSync(process.execPath, [cli, 'signer', 'generate', '--ci', '-p', '', '-w', key], { stdio: 'pipe' });
  writeFileSync(payload, 'interoperability fixture');
  execFileSync(
    process.execPath,
    [cli, 'signer', 'sign', '-f', key, '-p', '', '--app-version', '1.0.0', payload],
    { stdio: 'pipe' },
  );
  verifyUpdateSignature(
    readFileSync(payload),
    readFileSync(`${payload}.sig`, 'utf8'),
    readFileSync(`${key}.pub`, 'utf8'),
    '1.0.0',
  );
});

test('CLI preparation refuses to modify a local checkout', () => {
  assert.throws(
    () =>
      execFileSync(process.execPath, ['scripts/release.mjs', 'prepare', 'macos-arm64'], {
        env: { ...process.env, RELEASE_TAG: tag, GITHUB_ACTIONS: 'false' },
        stdio: 'pipe',
      }),
    (error) => {
      assert.match(String(error.stderr), /only allowed in a disposable/);
      return true;
    },
  );
});
