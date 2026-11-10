import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';

// rustup's standard location also works when the launching GUI has a minimal PATH.
const rustBin = path.join(homedir(), '.cargo', 'bin');
const env = { ...process.env, PATH: [rustBin, process.env.PATH ?? ''].join(path.delimiter) };
const [tool, ...args] = process.argv.slice(2);
const require = createRequire(import.meta.url);
let command, parameters;
if (tool === 'tauri') {
  if (args[0] === 'build' && !args.includes('--no-bundle') && !args.some((arg) => arg === '--bundles' || arg.startsWith('--bundles='))) {
    const bundle = process.platform === 'darwin' ? 'app' : process.platform === 'win32' ? 'nsis' : null;
    if (bundle) args.push('--bundles', bundle);
  }
  command = process.execPath;
  parameters = [require.resolve('@tauri-apps/cli/tauri.js'), ...args];
} else if (tool === 'cargo') {
  const bundled = path.join(rustBin, process.platform === 'win32' ? 'cargo.exe' : 'cargo');
  command = existsSync(bundled) ? bundled : 'cargo';
  parameters = args;
} else {
  throw new Error('Expected tauri or cargo');
}
const child = spawn(command, parameters, { env, stdio: 'inherit' });
child.on('error', (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on('exit', (code) => {
  process.exitCode = code ?? 1;
});
