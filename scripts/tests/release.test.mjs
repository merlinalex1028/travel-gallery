import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import {
  assetName,
  collectArtifact,
  platforms,
  releaseVersion,
  signingConfig,
  synchronizeVersion,
  verifyReleaseArtifacts,
} from '../release.mjs';

const commit = 'a'.repeat(40);
const tag = 'v1.2.3-beta.1';

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
    collectArtifact(directory, tag, commit, platform);
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
  const command = signingConfig(directory, 'windows-x64').bundle.windows.signCommand;
  assert.equal(command.cmd, 'pwsh');
  assert.deepEqual(command.args, [
    '-NoProfile',
    '-NonInteractive',
    '-File',
    path.join(directory, 'scripts/sign-windows.ps1'),
    '-FilePath',
    '%1',
  ]);
  assert.equal(signingConfig(directory, 'macos-arm64').bundle.macOS.hardenedRuntime, true);
  assert.throws(() => signingConfig(directory, 'windows-arm64'));
});

test('requires both platform artifacts from the same tag/commit and checks file contents', (t) => {
  const directory = fixture(t);
  const output = installers(directory);
  assert.equal(verifyReleaseArtifacts(output, tag, commit).length, 5);
  const sums = readFileSync(path.join(output, 'SHA256SUMS.txt'), 'utf8');
  assert.equal(sums.trim().split('\n').length, 4);
  assert.ok(sums.includes(assetName(tag, 'macos-arm64')));
  assert.throws(() => verifyReleaseArtifacts(output, tag, 'b'.repeat(40)), /provenance/);
  writeFileSync(path.join(output, assetName(tag, 'windows-x64')), 'tampered installer');
  assert.throws(() => verifyReleaseArtifacts(output, tag, commit), /provenance/);
});

test('rejects a missing platform, mixed receipt target, ambiguous or empty installers', (t) => {
  const directory = fixture(t);
  const output = installers(directory);
  const receiptPath = path.join(output, 'windows-x64.json');
  const receipt = JSON.parse(readFileSync(receiptPath, 'utf8'));
  receipt.target = 'aarch64-pc-windows-msvc';
  writeFileSync(receiptPath, JSON.stringify(receipt));
  assert.throws(() => verifyReleaseArtifacts(output, tag, commit), /provenance/);
  rmSync(receiptPath);
  assert.throws(() => verifyReleaseArtifacts(output, tag, commit), /ENOENT/);
  const spec = platforms['macos-arm64'];
  const folder = `src-tauri/target/${spec.target}/release/bundle/dmg/`;
  write(directory, `${folder}stale.dmg`, 'stale');
  assert.throws(() => collectArtifact(directory, tag, commit, 'macos-arm64'), /Expected one/);
  rmSync(path.join(directory, folder, 'stale.dmg'));
  write(directory, `${folder}installer.dmg`, '');
  assert.throws(() => collectArtifact(directory, tag, commit, 'macos-arm64'), /empty/);
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
