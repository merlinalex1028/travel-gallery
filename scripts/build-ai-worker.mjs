import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const env = {
  ...process.env,
  PATH: [path.join(homedir(), '.cargo/bin'), process.env.PATH ?? ''].join(path.delimiter),
};
const host = execFileSync('rustc', ['-vV'], { env, encoding: 'utf8' }).match(/^host: (.+)$/m)?.[1];
const target = process.env.TAURI_ENV_TARGET_TRIPLE ?? process.env.CARGO_BUILD_TARGET ?? host;
if (!target || !/^[\w.-]+$/.test(target)) throw new Error('Cannot identify the native sidecar target.');
const debug = process.argv.includes('--debug');
const args = [
  'build',
  '--manifest-path',
  path.join(root, 'src-tauri/Cargo.toml'),
  '--features',
  'ai-worker,quality-worker',
  '--bin',
  'travel-gallery-ai',
  '--bin',
  'travel-gallery-quality',
];
if (!debug) args.push('--release');
if (target !== host) args.push('--target', target);
execFileSync('cargo', args, { env, stdio: 'inherit', cwd: root });
const targetDir = process.env.CARGO_TARGET_DIR
  ? path.resolve(root, process.env.CARGO_TARGET_DIR)
  : path.join(root, 'src-tauri/target');
const extension = target.includes('windows') ? '.exe' : '';
for (const name of ['travel-gallery-ai', 'travel-gallery-quality']) {
  const built = path.join(
    targetDir,
    ...(target !== host || process.env.CARGO_BUILD_TARGET ? [target] : []),
    debug ? 'debug' : 'release',
    `${name}${extension}`,
  );
  const destination = path.join(root, 'src-tauri/binaries', `${name}-${target}${extension}`);
  mkdirSync(path.dirname(destination), { recursive: true });
  copyFileSync(built, destination);
}
console.log(
  `Prepared local CPU workers for ${target} (${debug ? 'debug' : 'release'}). No model weights included.`,
);
