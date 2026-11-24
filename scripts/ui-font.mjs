import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const asset = (name) => new URL(`../src/assets/fonts/${name}`, import.meta.url);
const metadata = JSON.parse(readFileSync(asset('metadata.json'), 'utf8'));

export function uiFontNotice() {
  for (const [name, expected] of Object.entries(metadata.files)) {
    const bytes = readFileSync(asset(name));
    const sha256 = createHash('sha256').update(bytes).digest('hex');
    if (bytes.length !== expected.bytes || sha256 !== expected.sha256)
      throw new Error(`Bundled UI font changed: ${name}`);
  }
  return (
    'BUNDLED OFFLINE UI FONT\n' +
    `${metadata.name} | ${metadata.license}\n` +
    `${metadata.files['Nunito.ttf'].sourceUrl}\n` +
    `SHA-256 ${metadata.files['Nunito.ttf'].sha256}\n\n` +
    readFileSync(asset('OFL.txt'), 'utf8')
      .replace(/[ \t]+$/gm, '')
      .trim() +
    '\nEND BUNDLED OFFLINE UI FONT\n\n'
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const notice = uiFontNotice();
  if (!readFileSync(new URL('../public/third-party-notices.txt', import.meta.url), 'utf8').includes(notice))
    throw new Error('Bundled UI font notice is missing or changed');
  console.log(JSON.stringify({ name: metadata.name, offline: true, hashesAndNoticeVerified: true }));
}
