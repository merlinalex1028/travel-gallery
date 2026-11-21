import { execFileSync } from 'node:child_process';
import { lstatSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

export const macNativePrograms = [
  'travel-gallery', 'travel-gallery-ai', 'travel-gallery-quality', 'ffmpeg', 'ffprobe',
];

function versionParts(version) {
  if (typeof version !== 'string' || !/^\d+(?:\.\d+){0,2}$/.test(version))
    throw new Error(`Invalid macOS deployment target: ${version}`);
  const parts = version.split('.').map(Number);
  return [parts[0], parts[1] ?? 0, parts[2] ?? 0];
}

function compareVersions(a, b) {
  const [major, minor, patch] = versionParts(a);
  const [otherMajor, otherMinor, otherPatch] = versionParts(b);
  return major - otherMajor || minor - otherMinor || patch - otherPatch;
}

export function assertMacNativeTargets(bundle, architecture, minimumSystemVersion) {
  const expectedArch = { arm64: 'arm64', x64: 'x86_64' }[architecture];
  if (!expectedArch) throw new Error(`Unsupported bundle architecture: ${architecture}`);
  const info = JSON.parse(execFileSync(
    '/usr/bin/plutil', ['-convert', 'json', '-o', '-', join(bundle, 'Contents', 'Info.plist')],
    { encoding: 'utf8' },
  ));
  if (info.CFBundleExecutable !== macNativePrograms[0])
    throw new Error(`Unexpected bundle executable: ${info.CFBundleExecutable}`);
  if (compareVersions(info.LSMinimumSystemVersion, minimumSystemVersion) !== 0)
    throw new Error(`Bundle minimum macOS version ${info.LSMinimumSystemVersion} differs from ${minimumSystemVersion}.`);
  return Object.fromEntries(macNativePrograms.map((name) => {
    const program = join(bundle, 'Contents', 'MacOS', name);
    const arches = execFileSync('/usr/bin/lipo', ['-archs', program], { encoding: 'utf8' }).trim();
    if (arches !== expectedArch) throw new Error(`${name}: expected ${expectedArch}, found ${arches}.`);
    const build = execFileSync('/usr/bin/vtool', ['-arch', expectedArch, '-show-build', program], { encoding: 'utf8' });
    const targets = [...build.matchAll(/^\s*platform MACOS\s*\n\s*minos (\S+)\s*$/gm)];
    if (targets.length !== 1) throw new Error(`${name}: expected one macOS deployment target.`);
    const [, target] = targets[0];
    if (compareVersions(target, minimumSystemVersion) > 0)
      throw new Error(`${name}: requires macOS ${target}, above bundle minimum ${minimumSystemVersion}.`);
    return [name, target];
  }));
}

export function assertFormalMacBundle(bundle) {
  if (!lstatSync(bundle).isDirectory()) throw new Error('Expected a real application directory.');
  const id = execFileSync(
    '/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleIdentifier', join(bundle, 'Contents', 'Info.plist')],
    { encoding: 'utf8' },
  ).trim();
  if (id !== 'com.travelgallery.desktop') {
    throw new Error(`Refusing to package a development or QA bundle (${id}).`);
  }
  const profileDirectories = new Set(['fixtures', 'fixture', 'thumbs', 'screenshots', 'test-results']);
  const profileFile = /\.(?:sqlite3?|db)(?:-(?:wal|shm|journal))?$|\.(?:safetensors|onnx|tflite|task)$/i;
  function check(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.isSymbolicLink() || profileDirectories.has(entry.name.toLowerCase()) || profileFile.test(entry.name)) {
        throw new Error(`Unexpected profile or test payload in the formal bundle: ${entry.name}`);
      }
      if (entry.isDirectory()) check(join(directory, entry.name));
    }
  }
  check(bundle);
}
