import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { brotliDecompressSync } from 'node:zlib';
import { mapFontNotice } from './map-resources.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const sha = (bytes) => createHash('sha256').update(bytes).digest('hex');
const read = (name) => readFileSync(resolve(root, name));
const json = (name) => JSON.parse(read(name));
const metadata = json('src/assets/map/metadata.json');
const archive = json('src/assets/map/world-tiles.json');

// This checks the current arm64 release's observed key/Brotli layout. It fails
// closed if another linker or Tauri version changes that layout.
function asset(binary, name) {
  const key = Buffer.from(name);
  const results = [];
  let offset = -1;
  while ((offset = binary.indexOf(key, offset + 1)) >= 0) {
    try {
      const result = brotliDecompressSync(binary.subarray(offset + key.length), {
        info: true,
        maxOutputLength: 16 * 1024 ** 2,
      });
      if (result.buffer.length)
        results.push({
          name,
          offset: offset + key.length,
          compressedBytes: result.engine.bytesWritten,
          bytes: result.buffer,
        });
    } catch {
      // A code string containing a URL alone is not an embedded resource.
    }
  }
  assert.equal(results.length, 1, `Missing or ambiguous embedded asset: ${name}`);
  return results[0];
}

function tilesFrom(page) {
  const entries = [...page.matchAll(/"([0-3]\/\d+\/\d+)":"([A-Za-z0-9+/]*={0,2})"/g)].map(
    (match) => [match[1], match[2]],
  );
  assert.equal(new Set(entries.map(([key]) => key)).size, entries.length, 'Duplicate tile keys');
  const tiles = Object.fromEntries(entries);
  assert.deepEqual(tiles, archive.tiles, 'Packaged tile keys or payloads differ');
  assert(page.includes('travel-map') && page.includes('://world/{z}/{x}/{y}.pbf'));
  assert.equal((page.match(/(?:https?|mapbox|pmtiles):\/\//g) ?? []).length, 0);
  return tiles;
}

function fontMatches(bytes) {
  const expected = metadata.font.files['NotoSans.ttf'];
  assert.equal(bytes.length, expected.bytes);
  assert.equal(sha(bytes), expected.sha256, 'Packaged font differs');
}

function stylesheetMatches(css, fontName) {
  assert(css.includes(`src:url(${fontName})`), 'Stylesheet does not use the packaged font');
  assert(css.includes('font-family:Travel Map Noto Sans'));
  for (const [, url] of css.matchAll(/url\(([^)]+)\)/g))
    assert(/^(?:["']?data:|\/assets\/|#)/.test(url), 'External stylesheet resource URL');
}

function noticesMatch(bytes) {
  assert(bytes.equals(read('public/third-party-notices.txt')), 'Packaged notices differ');
  assert(bytes.toString().includes(mapFontNotice()), 'Bundled OFL notice is missing');
}

async function main() {
  assert.deepEqual(process.argv.slice(2).filter((value) => value !== '--self-test'), []);
  const release = json('artifacts/build-info.json');
  const zip = resolve(root, 'artifacts', release.file);
  assert.equal(sha(readFileSync(zip)), release.sha256);
  const members = execFileSync('/usr/bin/unzip', ['-Z1', zip], { encoding: 'utf8' })
    .split('\n')
    .filter((name) => name.endsWith('.app/Contents/MacOS/travel-gallery'));
  assert.equal(members.length, 1);
  const binary = execFileSync('/usr/bin/unzip', ['-p', zip, members[0]], {
    maxBuffer: 128 * 1024 ** 2,
  });
  assert.equal(binary.readUInt32LE(0), 0xfeedfacf, 'Expected a thin Mach-O');
  assert.equal(binary.readUInt32LE(4), 0x0100000c, 'Expected arm64');
  assert.equal(sha(binary), release.executableSha256);
  assert(binary.equals(read('src-tauri/target/release/bundle/macos/Travel Gallery.app/Contents/MacOS/travel-gallery')));
  const keys = [...new Set(binary.toString('latin1').match(/\/assets\/[\w-]+\.(?:js|css|ttf)\b/g))];
  const unique = (pattern) => {
    const names = keys.filter((name) => pattern.test(name));
    assert.equal(names.length, 1, `Ambiguous asset names: ${pattern}`);
    return asset(binary, names[0]);
  };
  const page = unique(/^\/assets\/MapPage-[\w-]+\.js$/);
  const font = unique(/^\/assets\/NotoSans-[\w-]+\.ttf$/);
  const css = unique(/^\/assets\/index-[\w-]+\.css$/);
  const notices = asset(binary, '/third-party-notices.txt');
  const pageText = page.bytes.toString();
  const html = asset(binary, '/index.html');
  assert(html.bytes.toString().includes(css.name), 'Entry page does not load the inspected stylesheet');
  const [, entryName] = html.bytes.toString().match(/<script[^>]+src="([^"]+\.js)"/) ?? [];
  assert(entryName, 'Missing local entry script');
  const entry = asset(binary, entryName);
  assert(entry.bytes.includes(Buffer.from(page.name.slice('/assets/'.length))), 'Map page is absent from the entry routes');
  const [, mapLibraryName] = pageText.match(/\.\/(maplibre-gl-[\w-]+\.js)/) ?? [];
  assert(mapLibraryName, 'Missing bundled map library reference');
  const mapLibrary = asset(binary, `/assets/${mapLibraryName}`);
  const tiles = tilesFrom(pageText);
  fontMatches(font.bytes);
  stylesheetMatches(css.bytes.toString(), font.name);
  noticesMatch(notices.bytes);
  assert.equal(sha(read('src/assets/world.geojson')), metadata.sourceSha256);
  assert.equal(archive.sourceSha256, metadata.sourceSha256);
  const mapRequire = createRequire(createRequire(import.meta.url).resolve('maplibre-gl/package.json'));
  const { VectorTile } = await import(pathToFileURL(mapRequire.resolve('@mapbox/vector-tile')));
  const { default: PbfReader } = await import(pathToFileURL(mapRequire.resolve('pbf')));
  let features = 0;
  for (const encoded of Object.values(tiles)) {
    const layer = new VectorTile(new PbfReader(Buffer.from(encoded, 'base64'))).layers.land;
    if (!layer) continue;
    assert.equal(layer.version, 2);
    assert.equal(layer.extent, metadata.extent);
    features += layer.length;
    for (let index = 0; index < layer.length; index++) layer.feature(index).loadGeometry();
  }
  const negativeChecks = [];
  if (process.argv.includes('--self-test')) {
    const reject = (name, check) => {
      assert.throws(check, undefined, name);
      negativeChecks.push(name);
    };
    const [key, value] = Object.entries(tiles)[0];
    const entry = `"${key}":"${value}"`;
    reject('missing tile', () => tilesFrom(page.bytes.toString().replace(entry, '')));
    reject('changed tile payload', () => tilesFrom(page.bytes.toString().replace(value, `${value}A`)));
    reject('duplicate tile key', () => tilesFrom(`${page.bytes}\n{${entry}}`));
    const damaged = Buffer.from(font.bytes);
    damaged[0] ^= 1;
    reject('changed font bytes', () => fontMatches(damaged));
    reject('remote font URL', () =>
      stylesheetMatches(`${css.bytes}\n@font-face{src:url(https://invalid.example/font.ttf)}`, font.name));
    reject('missing font license', () =>
      noticesMatch(Buffer.from(notices.bytes.toString().replace(mapFontNotice(), ''))));
    reject('asset name without compressed bytes', () => asset(Buffer.from(font.name), font.name));
  }
  const report = {
    atUtc: new Date().toISOString(),
    passed: true,
    zipSha256: release.sha256,
    executableSha256: release.executableSha256,
    archiveMember: members[0],
    standardExecutableMatches: true,
    assets: [html, entry, page, mapLibrary, font, css, notices].map(({ name, offset, compressedBytes, bytes }) => ({
      name, offset, compressedBytes, bytes: bytes.length, sha256: sha(bytes),
    })),
    tileCount: Object.keys(tiles).length,
    decodedTileFeatures: features,
    sourceSha256: metadata.sourceSha256,
    fontAndLicenseMatch: true,
    negativeChecks,
    checkerSha256: sha(read('scripts/check-packaged-offline-map.mjs')),
    temporaryFilesGenerated: 0,
    limits: [
      'Static inspection of the current arm64 ZIP and embedded resources; no App or GUI was launched.',
      'Does not prove zero runtime requests, OS service attribution, physical frames, or legal release clearance.',
      'No extracted App, font, tile or notice copies are retained.',
    ],
  };
  writeFileSync(resolve(root, 'docs/current-packaged-map-check.json'), `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify({ passed: true, tileCount: report.tileCount, assets: report.assets.length, negativeChecks }));
}

await main();
