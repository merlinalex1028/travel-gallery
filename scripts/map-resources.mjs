import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const asset = (name) => resolve(root, 'src/assets/map', name);
const sha = (bytes) => createHash('sha256').update(bytes).digest('hex');
const metadata = JSON.parse(readFileSync(asset('metadata.json')));
const noticeStart = 'BUNDLED OFFLINE MAP FONT\n';
const noticeEnd = '\nEND BUNDLED OFFLINE MAP FONT\n\n';

export function mapFontNotice() {
  for (const [name, expected] of Object.entries(metadata.font.files)) {
    const bytes = readFileSync(asset(name));
    if (bytes.length !== expected.bytes || sha(bytes) !== expected.sha256)
      throw new Error(`Offline font resource changed: ${name}`);
  }
  return (
    noticeStart +
    `${metadata.font.name} | ${metadata.font.license}\n` +
    `${metadata.font.files['NotoSans.ttf'].sourceUrl}\n` +
    `SHA-256 ${metadata.font.files['NotoSans.ttf'].sha256}\n\n` +
    readFileSync(asset('OFL.txt'), 'utf8').trim() +
    noticeEnd
  );
}

async function buildTiles() {
  const mapRequire = createRequire(createRequire(import.meta.url).resolve('maplibre-gl/package.json'));
  const geoRoot = resolve(dirname(mapRequire.resolve('@maplibre/geojson-vt')), '..');
  const geoPackage = JSON.parse(readFileSync(resolve(geoRoot, 'package.json')));
  const { GeoJSONVT } = await import(pathToFileURL(resolve(geoRoot, geoPackage.module)));
  const { fromGeojsonVt } = await import(pathToFileURL(mapRequire.resolve('@maplibre/vt-pbf')));
  const { VectorTile } = await import(pathToFileURL(mapRequire.resolve('@mapbox/vector-tile')));
  const { default: PbfReader } = await import(pathToFileURL(mapRequire.resolve('pbf')));
  const bytes = readFileSync(resolve(root, 'src/assets/world.geojson'));
  if (sha(bytes) !== metadata.sourceSha256) throw new Error('World source checksum changed');
  const index = new GeoJSONVT(JSON.parse(bytes), {
    maxZoom: metadata.maxZoom,
    indexMaxZoom: metadata.maxZoom,
    indexMaxPoints: 0,
    extent: metadata.extent,
    buffer: 64,
    tolerance: 0,
  });
  const tiles = {};
  for (let z = 0; z <= metadata.maxZoom; z++) {
    for (let x = 0; x < 2 ** z; x++) {
      for (let y = 0; y < 2 ** z; y++) {
        const tile = index.getTile(z, x, y) ?? { features: [] };
        const encoded = fromGeojsonVt({ land: tile }, { version: 2, extent: metadata.extent });
        const decoded = new VectorTile(new PbfReader(encoded)).layers.land;
        if (
          (!decoded && tile.features.length !== 0) ||
          (decoded &&
            (decoded.version !== 2 ||
              decoded.extent !== metadata.extent ||
              decoded.length !== tile.features.length))
        )
          throw new Error(`Invalid generated MVT at ${z}/${x}/${y}`);
        tiles[`${z}/${x}/${y}`] = Buffer.from(encoded).toString('base64');
      }
    }
  }
  return (
    JSON.stringify({
      formatVersion: 1,
      sourceSha256: metadata.sourceSha256,
      maxZoom: metadata.maxZoom,
      extent: metadata.extent,
      tiles,
    }) + '\n'
  );
}

async function main() {
  if (!['--write', '--check'].includes(process.argv[2]) || process.argv.length !== 3)
    throw new Error('Usage: node scripts/map-resources.mjs --write|--check');
  const expected = await buildTiles();
  const fontNotice = mapFontNotice();
  const noticesPath = resolve(root, 'public/third-party-notices.txt');
  const notices = readFileSync(noticesPath, 'utf8');
  if (process.argv[2] === '--write') {
    writeFileSync(asset('world-tiles.json'), expected);
    const start = notices.indexOf(noticeStart);
    if (start >= 0) {
      const end = notices.indexOf(noticeEnd, start);
      if (end < 0) throw new Error('Incomplete existing font notice');
      writeFileSync(
        noticesPath,
        notices.slice(0, start) + fontNotice + notices.slice(end + noticeEnd.length),
      );
    } else {
      const table = notices.search(/^(?:npm|cargo|native):/m);
      if (table < 0) throw new Error('Component table is missing');
      writeFileSync(noticesPath, notices.slice(0, table) + fontNotice + notices.slice(table));
    }
  } else {
    if (readFileSync(asset('world-tiles.json'), 'utf8') !== expected)
      throw new Error('Generated offline tiles differ from the source');
    if (!notices.includes(fontNotice) || notices.match(/^BUNDLED OFFLINE MAP FONT$/gm)?.length !== 1)
      throw new Error('Bundled font notice is missing or duplicated');
  }
  console.log(
    JSON.stringify({
      tileCount: Object.keys(JSON.parse(expected).tiles).length,
      tileBytes: Buffer.byteLength(expected),
      maxZoom: metadata.maxZoom,
      fontBytes: metadata.font.files['NotoSans.ttf'].bytes,
      sourceSha256: metadata.sourceSha256,
      checked: process.argv[2] === '--check',
    }),
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
