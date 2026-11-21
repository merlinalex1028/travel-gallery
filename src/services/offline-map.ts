import archive from '../assets/map/world-tiles.json';
import type { AddProtocolAction, VectorSourceSpecification } from 'maplibre-gl';

export const offlineMapProtocol = 'travel-map';
export const offlineWorldSource: VectorSourceSpecification = {
  type: 'vector',
  tiles: [`${offlineMapProtocol}://world/{z}/{x}/{y}.pbf`],
  minzoom: 0,
  maxzoom: archive.maxZoom,
};

export const loadOfflineMapTile: AddProtocolAction = async ({ url }) => {
  const match = /^travel-map:\/\/world\/([0-3])\/(\d+)\/(\d+)\.pbf$/.exec(url);
  if (!match) throw new Error('离线地图瓦片地址无效');
  const [z, x, y] = match.slice(1).map(Number) as [number, number, number];
  if (x >= 2 ** z || y >= 2 ** z) throw new Error('离线地图瓦片超出范围');
  const encoded = (archive.tiles as Record<string, string>)[`${z}/${x}/${y}`];
  if (!encoded) throw new Error('离线地图瓦片缺失');
  // A fresh buffer may be transferred to MapLibre's worker without detaching the archive.
  const bytes = Uint8Array.from(atob(encoded), (character) => character.charCodeAt(0));
  return { data: bytes.buffer };
};
