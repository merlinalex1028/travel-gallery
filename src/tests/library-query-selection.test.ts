import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useLibrary } from '../stores/library';
import type { AssetSummary, QueryAssetsResult, SelectionSnapshot } from '../services/contracts';

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
it('freezes only the catalog version shown in the gallery and retains the previous scope if it changed', async () => {
  const lib = oldResult();
  lib.snapshot = { id: 'frozen', count: 2, createdAt: 1, expiresAt: 100, queryFingerprint: 'old-query' };
  lib.excluded = new Set(['excluded-member']);
  invoke.mockResolvedValueOnce({
    ...page('visible', 'old-query'),
    catalogEpoch: 42,
    selectionMembership: { snapshotId: 'frozen', assetIds: ['visible'] },
  });
  await lib.search();
  const stale = { code: 'INVALID_FILTER', message: '资料库已变化，请刷新结果后重新选择' };
  invoke.mockRejectedValueOnce(stale);
  await expect(lib.selectAllMatching()).rejects.toEqual(stale);
  expect(invoke).toHaveBeenLastCalledWith('create_selection_snapshot', {
    filter: {}, sort: { field: 'captureUtcMs', direction: 'desc' }, expectedCatalogEpoch: 42,
  });
  expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen', excludeAssetIds: ['excluded-member'] });
});
function deferNext<T>() {
  let resolve!: (result: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  invoke.mockReturnValueOnce(promise);
  return { resolve, reject };
}
function page(id: string, queryFingerprint: string, nextCursor: string | null = null): QueryAssetsResult {
  return { items: [{ id } as AssetSummary], total: 2, nextCursor, queryFingerprint, catalogEpoch: 1 };
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

it('discards an old page as soon as the filter changes and only pages from the new result', async () => {
  const lib = oldResult();
  const stale = deferNext<QueryAssetsResult>();
  const paging = lib.search(true);
  expect(lib.loadingMore).toBe(true);
  lib.setFilter({ keyword: 'fresh' });
  stale.resolve(page('old-page', 'old-query', 'old-next'));
  await paging;
  expect(lib.items.map(({ id }) => id)).toEqual(['old-video']);
  expect(lib.total).toBe(1);
  expect(lib.cursor).toBeNull();
  expect(lib.loadingMore).toBe(false);
  expect(lib.queryUnavailable).toBe(true);
  await lib.search(true);
  lib.selectLoaded();
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(lib.count).toBe(0);

  invoke.mockResolvedValueOnce(page('fresh-first', 'fresh-query', 'fresh-next'));
  await lib.search();
  expect(lib.queryUnavailable).toBe(false);
  expect(lib.fingerprint).toBe('fresh-query');
  invoke.mockResolvedValueOnce(page('fresh-second', 'fresh-query'));
  await lib.search(true);
  expect(lib.items.map(({ id }) => id)).toEqual(['fresh-first', 'fresh-second']);
  expect(invoke).toHaveBeenLastCalledWith('query_assets', expect.objectContaining({ cursor: 'fresh-next' }));
});

it('ignores an old page error after a sort change and keeps a newer first page busy', async () => {
  const lib = oldResult();
  const stale = deferNext<QueryAssetsResult>();
  const paging = lib.search(true);
  lib.sort.direction = 'asc';
  const current = deferNext<QueryAssetsResult>();
  const refresh = lib.search();
  stale.reject({ code: 'INVALID_FILTER' });
  await paging;
  expect(lib.error).toBe('');
  expect(lib.loadingMore).toBe(false);
  expect(lib.loading).toBe(true);
  current.resolve(page('ascending', 'ascending-query'));
  await refresh;
  expect(lib.queryUnavailable).toBe(false);
  expect(lib.items.map(({ id }) => id)).toEqual(['ascending']);
});

it('ignores an old page error before a new first page starts', async () => {
  const lib = oldResult();
  const stale = deferNext<QueryAssetsResult>();
  const paging = lib.search(true);
  lib.sort.direction = 'asc';
  stale.reject({ code: 'INVALID_FILTER' });
  await paging;
  expect(lib.error).toBe('');
  expect(lib.queryUnavailable).toBe(true);
});

it.each(['resolve', 'reject'] as const)(
  'discards a stale select-all %s and keeps the frozen scope',
  async (outcome) => {
    const lib = oldResult();
    lib.snapshot = { id: 'frozen', count: 2, createdAt: 1, expiresAt: 100, queryFingerprint: 'old-query' };
    lib.excluded = new Set(['excluded-member']);
    const target = lib.target;
    const stale = deferNext<SelectionSnapshot>();
    const selecting = lib.selectAllMatching();
    lib.setFilter({ sourceIds: ['changed'] });
    if (outcome === 'resolve') stale.resolve({ ...lib.snapshot, id: 'old-replacement', count: 5 });
    else stale.reject({ code: 'INVALID_FILTER' });
    await selecting;
    expect(lib.target).toEqual(target);
    expect(invoke).toHaveBeenCalledTimes(1);
  },
);
