import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

if (process.platform !== 'win32' || process.arch !== 'x64')
  throw new Error(
    'Windows packaging requires a native Windows x64 host; no other platform package was built.',
  );
const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const runner = process.env.npm_execpath;
if (!runner) throw new Error('Run with pnpm desktop:package:windows.');
const node = (script, ...args) =>
  execFileSync(process.execPath, [path.join(root, script), ...args], { cwd: root, stdio: 'inherit' });
const pnpm = (...args) =>
  execFileSync(
    /\.(?:c?js|mjs)$/.test(runner) ? process.execPath : runner,
    /\.(?:c?js|mjs)$/.test(runner) ? [runner, ...args] : args,
    { cwd: root, stdio: 'inherit' },
  );
const started = Date.now();
node('scripts/build-media-tools.mjs');
const smoke = path.join(root, 'artifacts', `windows-media-smoke-${started}.json`);
node('scripts/smoke-media-tools.mjs', '--output', smoke);
pnpm('notices');
pnpm('desktop:build');
const metadata = JSON.parse(
  execFileSync(
    process.execPath,
    [
      path.join(root, 'scripts/native.mjs'),
      'cargo',
      'metadata',
      '--manifest-path',
      path.join(root, 'src-tauri/Cargo.toml'),
      '--format-version',
      '1',
      '--no-deps',
    ],
    { cwd: root, encoding: 'utf8' },
  ),
);
const release = path.join(
  metadata.target_directory,
  ...(process.env.CARGO_BUILD_TARGET ? [process.env.CARGO_BUILD_TARGET] : []),
  'release',
);
const installers = path.join(release, 'bundle', 'nsis');
const files = readdirSync(installers).filter(
  (name) => name.endsWith('.exe') && statSync(path.join(installers, name)).mtimeMs >= started - 2000,
);
if (files.length !== 1)
  throw new Error('Expected exactly one newly built NSIS installer; no completion receipt written.');
const stamp = (file) => ({
  path: file,
  bytes: statSync(file).size,
  sha256: createHash('sha256').update(readFileSync(file)).digest('hex'),
});
const binaries = { 'travel-gallery': stamp(path.join(release, 'travel-gallery.exe')) };
for (const name of ['travel-gallery-ai', 'travel-gallery-quality', 'ffmpeg', 'ffprobe'])
  binaries[name] = stamp(path.join(root, 'src-tauri/binaries', `${name}-x86_64-pc-windows-msvc.exe`));
const receipt = {
  atUtc: new Date().toISOString(),
  platform: 'Windows',
  architecture: 'x64',
  installer: stamp(path.join(installers, files[0])),
  bundleInputBinaries: binaries,
  mediaSmokeReport: smoke,
  mediaTools: JSON.parse(
    readFileSync(path.join(root, 'src-tauri/resources/media-tools/build-info.json'), 'utf8'),
  ),
  webviewMode: 'Tauri offlineInstaller; WebView2 is acquired at build time',
  scope:
    'Native build, local CLI smoke and installer/bundle-input checksums; extraction, installer launch/upgrade/uninstall, offline runtime and release signing remain separate acceptance gates.',
};
mkdirSync(path.join(root, 'artifacts'), { recursive: true });
writeFileSync(path.join(root, 'artifacts/windows-build-info.json'), JSON.stringify(receipt, null, 2) + '\n');
console.log(`Built Windows installer: ${receipt.installer.path}; SHA-256 ${receipt.installer.sha256}`);
