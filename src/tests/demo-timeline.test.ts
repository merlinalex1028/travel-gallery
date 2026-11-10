import { afterEach, expect, it, vi } from 'vitest';
import golden from '../../tests/fixtures/contracts.json';
import type { AssetDetail, Commands } from '../services/contracts';
afterEach(() => localStorage.removeItem('travel-gallery-demo-v1'));
it('applies trip filters and the trusted-date split in the browser adapter', async () => {
  const assets = ['known', 'mtime', 'outside', 'invalid'].map(
    (id) => ({ ...structuredClone(golden.assetDetail), id, filename: `${id}.jpg` }) as AssetDetail,
  );
  assets[0]!.captureTzMin = null;
  assets[1]!.captureTimeSource = 'mtime';
  assets[3]!.captureLocal = '2026-02-30 12:00:00';
  localStorage.setItem(
    'travel-gallery-demo-v1',
    JSON.stringify({
      assets,
      tags: [],
      trips: [],
      tripAssets: { trip: ['known', 'mtime', 'invalid'] },
      settings: {},
      saved: [],
    }),
  );
  vi.resetModules();
  const { demoCall } = await import('../services/demo');
  const result = (await demoCall('timeline_stats', {
    filter: { collectionId: 'trip' },
  })) as Commands['timeline_stats']['result'];
  expect(result).toEqual([
    { date: '2026-06-15', count: 1, reviewed: 0, unknownTimezone: 1, photos: 1, videos: 0 },
    { date: null, count: 2, reviewed: 0, unknownTimezone: 0, photos: 2, videos: 0 },
  ]);
  const page = (await demoCall('query_assets', {
    filter: { collectionId: 'trip', captureDateKnown: false },
    sort: { field: 'filename', direction: 'asc' },
    pageSize: 120,
    cursor: null,
  })) as Commands['query_assets']['result'];
  expect(page.items.map((a) => a.id).sort()).toEqual(['invalid', 'mtime']);
});
it('reads story chapters from actual demo membership while requiring native authority for export', async () => {
  const assets = ['wall', 'invalid', 'outside'].map(
    (id) => ({ ...structuredClone(golden.assetDetail), id, filename: `${id}.jpg` }) as AssetDetail,
  );
  assets[0]!.captureTzMin = null;
  assets[0]!.review.pickState = 'keep';
  assets[1]!.captureLocal = '2026-02-30 12:00:00';
  localStorage.setItem(
    'travel-gallery-demo-v1',
    JSON.stringify({
      assets,
      tags: [],
      trips: [{ id: 'trip', name: '旅行', count: 2 }],
      tripAssets: { trip: ['wall', 'invalid'] },
      settings: {},
      saved: [],
    }),
  );
  vi.resetModules();
  const { demoCall } = await import('../services/demo');
  const request = { tripId: 'trip', range: 'all' as const, offset: 0, pageSize: 100, epoch: null };
  const result = (await demoCall('get_trip_story', request)) as Commands['get_trip_story']['result'];
  expect(result.items.map((a) => [a.assetId, a.date])).toEqual([
    ['wall', '2026-06-15'],
    ['invalid', null],
  ]);
  expect(result.items[0]!.unknownTimezone).toBe(true);
  expect(
    (
      (await demoCall('get_trip_story', {
        ...request,
        range: 'keep',
      })) as Commands['get_trip_story']['result']
    ).total,
  ).toBe(1);
  await expect(demoCall('preview_story_export', {})).rejects.toBeTruthy();
  await expect(demoCall('start_story_export', {})).rejects.toBeTruthy();
});
