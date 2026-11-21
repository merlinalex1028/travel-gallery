import { afterEach, expect, it, vi } from 'vitest';
import { createRequire } from 'node:module';
import { loadOfflineMapTile, offlineWorldSource } from '../services/offline-map';
import archive from '../assets/map/world-tiles.json';
import world from '../assets/world.geojson?raw';

const mapRequire = createRequire(createRequire(import.meta.url).resolve('maplibre-gl/package.json'));
const { VectorTile } = mapRequire('@mapbox/vector-tile');
const Pbf = mapRequire('pbf').default;
afterEach(() => vi.unstubAllGlobals());

async function tile(z: number, x: number, y: number) {
  const result = await loadOfflineMapTile(
    { url: `travel-map://world/${z}/${x}/${y}.pbf` },
    new AbortController(),
  );
  return result.data as ArrayBuffer;
}

it('serves all 85 packaged tiles without a network read and returns transferable copies', async () => {
  const fetcher = vi.fn();
  vi.stubGlobal('fetch', fetcher);
  expect(offlineWorldSource).toMatchObject({ type: 'vector', minzoom: 0, maxzoom: 3 });
  expect(Object.keys(archive.tiles)).toHaveLength(85);
  for (const key of Object.keys(archive.tiles)) {
    const [z, x, y] = key.split('/').map(Number) as [number, number, number];
    const data = await tile(z, x, y);
    expect(data.byteLength).toBeGreaterThan(0);
    const decoded = new VectorTile(new Pbf(new Uint8Array(data))).layers.land;
    if (decoded) {
      expect(decoded.version).toBe(2);
      expect(decoded.extent).toBe(4096);
      for (let index = 0; index < decoded.length; index++) expect(decoded.feature(index).type).toBe(3);
    }
  }
  const first = new Uint8Array(await tile(0, 0, 0));
  const previous = first[0];
  first[0] = previous! ^ 255;
  expect(new Uint8Array(await tile(0, 0, 0))[0]).toBe(previous);
  expect(fetcher).not.toHaveBeenCalled();
});

it.each([
  'https://tiles.example/0/0/0.pbf',
  'travel-map://other/0/0/0.pbf',
  'travel-map://world/4/0/0.pbf',
  'travel-map://world/3/8/0.pbf',
  'travel-map://world/3/0/8.pbf',
  'travel-map://world/0/../0.pbf',
  'travel-map://world/0/0/0.pbf?url=https://tiles.example',
])('rejects nonlocal or out-of-range tile address %s', async (url) => {
  const fetcher = vi.fn();
  vi.stubGlobal('fetch', fetcher);
  await expect(loadOfflineMapTile({ url }, new AbortController())).rejects.toThrow('离线地图');
  expect(fetcher).not.toHaveBeenCalled();
});

type Position = [number, number];
function inRing([x, y]: Position, ring: Position[]) {
  let inside = false;
  for (let index = 0, previous = ring.length - 1; index < ring.length; previous = index++) {
    const [a, b] = ring[index]!,
      [c, d] = ring[previous]!;
    if (b > y !== d > y && x < ((c - a) * (y - b)) / (d - b) + a) inside = !inside;
  }
  return inside;
}
function contains(geometry: { type: string; coordinates: Position[][] | Position[][][] }, point: Position) {
  const polygons = (
    geometry.type === 'Polygon' ? [geometry.coordinates] : geometry.coordinates
  ) as Position[][][];
  return polygons.some(
    (polygon) => inRing(point, polygon[0]!) && !polygon.slice(1).some((ring) => inRing(point, ring)),
  );
}

it('preserves land and ocean locations from the original geometry at three zoom levels', async () => {
  const source = JSON.parse(world);
  for (const [lon, lat, land] of [
    [2.34, 48.86, true],
    [116.4, 39.9, true],
    [-104.99, 39.74, true],
    [151.21, -33.86, true],
    [-30, 30, false],
    [-140, 0, false],
  ] as const) {
    const point: Position = [lon, lat];
    expect(
      source.features.some((feature: { geometry: Parameters<typeof contains>[0] }) =>
        contains(feature.geometry, point),
      ),
    ).toBe(land);
    for (const z of [0, 1, 3]) {
      const latitude = (lat * Math.PI) / 180;
      const x = Math.floor(((lon + 180) / 360) * 2 ** z);
      const y = Math.floor(
        ((1 - Math.log(Math.tan(latitude) + 1 / Math.cos(latitude)) / Math.PI) / 2) * 2 ** z,
      );
      const layer = new VectorTile(new Pbf(new Uint8Array(await tile(z, x, y)))).layers.land;
      let found = false;
      for (let index = 0; layer && index < layer.length; index++)
        found ||= contains(layer.feature(index).toGeoJSON(x, y, z).geometry, point);
      expect(found, `${lon},${lat} z${z}`).toBe(land);
    }
  }
});
