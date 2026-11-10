import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useLibrary } from '../stores/library';
import type { AssetSummary } from '../services/contracts';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: () => '范围起点不能大于终点' }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), error: vi.fn() } }));
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
});
function oldResult() {
  const lib = useLibrary();
  lib.items = [{ id: 'old-video' } as AssetSummary];
  lib.total = 1;
  lib.cursor = 'old-cursor';
  lib.fingerprint = 'old-query';
  return lib;
}
it('blocks selecting cached rows or paging after a new query fails, then permits the recovered results', async () => {
  const lib = oldResult();
  lib.setFilter({ captureLocalFrom: '2023-03-24', captureLocalTo: '2023-03-23' });
  let rejectQuery!: (error: unknown) => void;
  invoke.mockImplementationOnce(
    () =>
      new Promise((_, reject) => {
        rejectQuery = reject;
      }),
  );
  const request = lib.search();
  try {
    expect(lib.loading).toBe(true);
    lib.selectLoaded();
    lib.select(lib.items[0]!);
    await lib.selectAllMatching();
    expect([...lib.selected]).toEqual([]);
    expect(invoke).toHaveBeenCalledTimes(1);
  } finally {
    rejectQuery({ code: 'INVALID_FILTER' });
    await request;
  }
  expect(lib.error).toBeTruthy();
  lib.selectLoaded();
  lib.select(lib.items[0]!);
  await lib.selectAllMatching();
  await lib.search(true);
  expect([...lib.selected]).toEqual([]);
  expect(invoke).toHaveBeenCalledTimes(1);

  lib.setFilter({ keyword: 'fresh' });
  invoke.mockResolvedValueOnce({
    items: [{ id: 'fresh-image' }],
    total: 1,
    nextCursor: null,
    queryFingerprint: 'fresh-query',
  });
  await lib.search();
  lib.selectLoaded();
  expect([...lib.selected]).toEqual(['fresh-image']);
  invoke.mockResolvedValueOnce({
    id: 'fresh-snapshot',
    count: 1,
    createdAt: 1,
    expiresAt: 100,
    queryFingerprint: 'fresh-query',
  });
  invoke.mockResolvedValueOnce({
    items: [{ id: 'fresh-image' }],
    total: 1,
    nextCursor: null,
    queryFingerprint: 'fresh-query',
    selectionMembership: { snapshotId: 'fresh-snapshot', assetIds: ['fresh-image'] },
  });
  await lib.selectAllMatching();
  expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'fresh-snapshot', excludeAssetIds: [] });
  expect(lib.error).toBe('');
  expect(invoke).toHaveBeenCalledWith('create_selection_snapshot', {
    filter: { keyword: 'fresh' },
    sort: { field: 'captureUtcMs', direction: 'desc' },
  });
});
it('keeps a previously frozen target and its exclusions when a new query is pending or rejected', async () => {
  const lib = oldResult();
  lib.snapshot = {
    id: 'known-frozen',
    count: 2,
    createdAt: 1,
    expiresAt: 100,
    queryFingerprint: 'old-query',
  };
  lib.excluded = new Set(['excluded-member']);
  const target = { ...lib.target };
  lib.setFilter({ keyword: 'changed' });
  let rejectQuery!: (error: unknown) => void;
  invoke.mockImplementationOnce(
    () =>
      new Promise((_, reject) => {
        rejectQuery = reject;
      }),
  );
  const request = lib.search();
  try {
    lib.selectLoaded();
    lib.select(lib.items[0]!);
    await lib.selectAllMatching();
    expect(lib.target).toEqual(target);
    expect(invoke).toHaveBeenCalledTimes(1);
  } finally {
    rejectQuery({ code: 'INVALID_FILTER' });
    await request;
  }
  lib.selectLoaded();
  lib.select(lib.items[0]!);
  await lib.selectAllMatching();
  expect(lib.target).toEqual(target);
  expect(invoke).toHaveBeenCalledTimes(1);
});
