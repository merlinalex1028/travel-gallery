import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const work = mkdtempSync(join(tmpdir(), 'travel-gallery-icons-'));
try {
  const logo = readFileSync(join(root, 'public/logo.svg'), 'utf8');
  const content = logo.replace(/^<svg\b[^>]*>/, '').replace(/<\/svg>\s*$/, '');
  // Keep the web mark edge-to-edge; macOS icons need optical padding in their canvas.
  const input = join(work, 'app-icon.svg');
  writeFileSync(
    input,
    `<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
      <g transform="translate(100 100) scale(1.609375)">${content}</g>
    </svg>`,
  );
  execFileSync(
    process.execPath,
    [
      join(root, 'scripts/native.mjs'),
      'tauri',
      'icon',
      input,
      '--output',
      join(root, 'src-tauri/icons'),
      '--ios-color',
      '#f0f4fa',
    ],
    { cwd: root, stdio: 'inherit' },
  );
} finally {
  rmSync(work, { recursive: true, force: true });
}
