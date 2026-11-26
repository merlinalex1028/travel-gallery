import { copyFile, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const output = path.join(root, 'dist/site');
const repository = 'https://github.com/merlinalex1028/travel-gallery';
const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== '--releases'))
  throw new Error('Usage: node scripts/build-site.mjs [--releases FILE]');

// An explicit source allowlist keeps desktop resources and local records out of Pages.
await mkdir(path.join(output, 'assets'), { recursive: true });
for (const name of ['index.html', 'styles.css', 'main.js'])
  await copyFile(path.join(root, 'site', name), path.join(output, name));
await copyFile(
  path.join(root, 'site/assets/library-preview.jpg'),
  path.join(output, 'assets/library-preview.jpg'),
);
await copyFile(path.join(root, 'public/logo.svg'), path.join(output, 'assets/logo.svg'));
await copyFile(path.join(root, 'public/demo/landscape-0.svg'), path.join(output, 'assets/journey.svg'));
await copyFile(path.join(root, 'src/assets/fonts/Nunito.ttf'), path.join(output, 'assets/Nunito.ttf'));
await copyFile(path.join(root, 'src/assets/fonts/OFL.txt'), path.join(output, 'assets/OFL.txt'));

const releases = args.length ? JSON.parse(await readFile(path.resolve(args[1]), 'utf8')) : [];
if (!Array.isArray(releases)) throw new Error('Expected the GitHub releases API array.');
const published = releases.filter((release) => !release.draft && release.published_at);
// Prefer the newest complete stable version; use a prerelease while no stable build exists.
const complete = published
  .filter((release) =>
    ['-macOS-arm64.dmg', '-Windows-x64-setup.exe'].every((suffix) =>
      release.assets?.some(
        (asset) => asset.name.endsWith(suffix) && asset.state === 'uploaded' && asset.size > 0,
      ),
    ),
  )
  .sort((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at));
const release = complete.find((item) => !item.prerelease) ?? complete[0];
const downloads = { url: `${repository}/releases`, tag: null, assets: {} };
if (release) {
  if (!release.html_url.startsWith(`${repository}/releases/tag/`)) throw new Error('Unexpected release URL.');
  downloads.url = release.html_url;
  downloads.tag = release.tag_name;
  downloads.prerelease = release.prerelease;
  for (const [platform, suffix] of [
    ['macos', '-macOS-arm64.dmg'],
    ['windows', '-Windows-x64-setup.exe'],
  ]) {
    const asset = release.assets.find((item) => item.name.endsWith(suffix));
    if (!asset.browser_download_url.startsWith(`${repository}/releases/download/`))
      throw new Error('Unexpected download destination.');
    downloads.assets[platform] = { name: asset.name, url: asset.browser_download_url, bytes: asset.size };
  }
}
await writeFile(path.join(output, 'downloads.json'), `${JSON.stringify(downloads, null, 2)}\n`);
await writeFile(path.join(output, '.nojekyll'), '');
const expected = ['.nojekyll', 'assets', 'downloads.json', 'index.html', 'main.js', 'styles.css'];
if (JSON.stringify((await readdir(output)).sort()) !== JSON.stringify(expected))
  throw new Error('Unexpected files in dist/site; remove the previous site output and rebuild.');
const assets = ['Nunito.ttf', 'OFL.txt', 'journey.svg', 'library-preview.jpg', 'logo.svg'];
if (JSON.stringify((await readdir(path.join(output, 'assets'))).sort()) !== JSON.stringify(assets))
  throw new Error('Unexpected website assets; refusing to publish files outside the allowlist.');
console.log(`Built GitHub Pages site: dist/site (${downloads.tag ?? 'release-page download links'}).`);
