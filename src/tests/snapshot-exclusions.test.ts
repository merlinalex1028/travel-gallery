import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { ElMessage } from 'element-plus';
import { useLibrary } from '../stores/library';
import SelectionBar from '../features/SelectionBar.vue';
import type { AssetSummary } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({
  ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
}));
const asset = (id: string) =>
  ({ ...structuredClone(golden.assetDetail), id, filename: `${id}.png` }) as AssetSummary;
let wrapper: VueWrapper | undefined;
let queryItems: AssetSummary[];
let membership: string[];

beforeEach(() => {
  setActivePinia(createPinia());
  queryItems = [asset('a'), asset('b')];
  membership = ['a', 'b'];
  invoke.mockReset();
  vi.mocked(ElMessage.info).mockClear();
  invoke.mockImplementation(async (name: string, request: Record<string, unknown>) => {
    if (name === 'create_selection_snapshot')
      return {
        id: 'frozen-five',
        count: 5,
        createdAt: 1,
        expiresAt: 9999999999999,
        queryFingerprint: 'query',
      };
    if (name === 'query_assets')
      return {
        items: queryItems,
        total: queryItems.length,
        nextCursor: null,
        queryFingerprint: (request.filter as { keyword?: string }).keyword ? 'changed-query' : 'query',
        selectionMembership: { snapshotId: 'frozen-five', assetIds: membership },
      };
    if (name === 'get_asset') return asset(String(request.assetId));
    if (name === 'apply_review_batch')
      return { changeBatchId: 'review-four', affectedCount: 4, unchangedCount: 0, conflicts: [] };
    throw new Error(name);
  });
});
afterEach(() => wrapper?.unmount());

async function freeze() {
  const lib = useLibrary();
  lib.items = queryItems;
  lib.fingerprint = 'query';
  await lib.selectAllMatching();
  await lib.search();
  return lib;
}

it.each(['metaKey', 'ctrlKey'] as const)(
  'toggles one frozen member with %s without replacing the snapshot',
  async (modifier) => {
    const lib = await freeze();
    lib.select(asset('a'), new MouseEvent('click', { [modifier]: true }));
    expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen-five', excludeAssetIds: ['a'] });
    expect(lib.count).toBe(4);
    expect(lib.isSelected('a')).toBe(false);
    expect(lib.isSelected('b')).toBe(true);
    wrapper = mount(SelectionBar, {
      global: {
        stubs: {
          Icon: true,
          ElDropdown: true,
          ElDropdownMenu: true,
          ElDropdownItem: true,
          ElDialog: true,
          ElSelect: true,
          ElOption: true,
          RouterLink: true,
        },
      },
    });
    expect(wrapper.text()).toContain('已排除 1 张');
    lib.select(asset('a'), new MouseEvent('click', { [modifier]: true }));
    await flushPromises();
    expect(lib.count).toBe(5);
    expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen-five', excludeAssetIds: [] });
  },
);

it('does not select or add a newly indexed row with the same filter fingerprint', async () => {
  const lib = await freeze();
  queryItems = [asset('a'), asset('new')];
  membership = ['a'];
  await lib.search();
  expect(lib.isSelected('a')).toBe(true);
  expect(lib.isSelected('new')).toBe(false);
  lib.select(asset('new'), new MouseEvent('click', { metaKey: true }));
  expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen-five', excludeAssetIds: [] });
  expect(lib.count).toBe(5);
  expect(ElMessage.info).toHaveBeenCalled();
});

it('keeps exclusions when a changed query is applied, including the target of an already queued edit', async () => {
  const lib = await freeze();
  lib.select(asset('a'), new MouseEvent('click', { metaKey: true }));
  lib.setFilter({ keyword: 'b' });
  queryItems = [asset('b')];
  membership = ['b'];
  await lib.search();
  const pending = lib.apply({ pickState: 'keep' });
  lib.select(asset('b'), new MouseEvent('click', { metaKey: true }));
  await pending;
  const request = invoke.mock.calls.find(([name]) => name === 'apply_review_batch')![1];
  expect(request.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen-five', excludeAssetIds: ['a'] });
  expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen-five', excludeAssetIds: ['a', 'b'] });
  expect(
    invoke.mock.calls.some(([name, r]) => name === 'query_assets' && r.selectionSnapshotId === 'frozen-five'),
  ).toBe(true);
});

it('preserves frozen membership across pagination without marking nonmembers', async () => {
  const lib = await freeze();
  lib.cursor = 'next-page';
  queryItems = [asset('c'), asset('new')];
  membership = ['c'];
  await lib.search(true);
  expect(lib.isSelected('a')).toBe(true);
  expect(lib.isSelected('c')).toBe(true);
  expect(lib.isSelected('new')).toBe(false);
});

it('keeps the snapshot when all currently loaded members are excluded and allows reinclusion', async () => {
  const lib = await freeze();
  lib.snapshot!.count = 2;
  for (const item of queryItems) lib.select(item, new MouseEvent('click', { ctrlKey: true }));
  expect(lib.count).toBe(0);
  expect(lib.target.kind).toBe('snapshot');
  lib.select(asset('b'), new MouseEvent('click', { ctrlKey: true }));
  expect(lib.count).toBe(1);
  expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen-five', excludeAssetIds: ['a'] });
});

it('still replaces the frozen selection on an ordinary click', async () => {
  const lib = await freeze();
  lib.select(asset('b'), new MouseEvent('click'));
  expect(lib.target).toEqual({ kind: 'explicit', assetIds: ['b'] });
  expect(lib.count).toBe(1);
});

it('blocks new selection edits when the page cannot confirm its snapshot membership', async () => {
  const lib = await freeze();
  invoke.mockResolvedValueOnce({
    items: [asset('new')],
    total: 1,
    nextCursor: null,
    queryFingerprint: 'query',
    selectionMembership: { snapshotId: 'different-snapshot', assetIds: ['new'] },
  });
  await lib.search();
  expect(lib.queryUnavailable).toBe(true);
  lib.select(asset('a'), new MouseEvent('click', { metaKey: true }));
  expect(lib.count).toBe(5);
  expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen-five', excludeAssetIds: [] });
});

it('does not revive a snapshot after the user cleared the pending selection', async () => {
  const lib = useLibrary();
  let finish!: (value: unknown) => void;
  invoke.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const creating = lib.selectAllMatching();
  lib.clearSelection();
  finish({ id: 'too-late', count: 5, createdAt: 1, expiresAt: 9999999999999, queryFingerprint: 'query' });
  await creating;
  expect(lib.target).toEqual({ kind: 'explicit', assetIds: [] });
  expect(invoke).toHaveBeenCalledTimes(1);
});

it('does not replace a newer exclusion edit with a late select-all response', async () => {
  const lib = await freeze();
  let finish!: (value: unknown) => void;
  invoke.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const creating = lib.selectAllMatching();
  lib.select(asset('a'), new MouseEvent('click', { metaKey: true }));
  finish({ id: 'too-late', count: 99, createdAt: 1, expiresAt: 9999999999999, queryFingerprint: 'query' });
  await creating;
  expect(lib.count).toBe(4);
  expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen-five', excludeAssetIds: ['a'] });
});
