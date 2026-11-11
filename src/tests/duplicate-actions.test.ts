import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import DuplicatesPage from '../pages/DuplicatesPage.vue';
import { useApp } from '../stores/app';
import type { AssetSummary, DuplicateGroup } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const { invoke, confirm } = vi.hoisted(() => ({ invoke: vi.fn(), confirm: vi.fn() }));
vi.mock('../services/api', () => ({
  api: invoke,
  isDemo: false,
  native: true,
  errorMessage: () => 'failed',
}));
vi.mock('element-plus', () => ({
  ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
  ElMessageBox: { confirm },
}));
let wrapper: VueWrapper | undefined;
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
  confirm.mockReset().mockResolvedValue('confirm');
});
afterEach(() => wrapper?.unmount());

async function showGroup(otherExcluded: boolean, stale = false, allExcluded = false) {
  const group: DuplicateGroup = {
    id: 'group',
    kind: 'exact',
    algorithm: 'SHA-256',
    algorithmVersion: '1',
    representativeAssetId: allExcluded ? null : 'representative',
    memberCount: allExcluded ? 0 : otherExcluded ? 1 : 2,
    totalMembers: 3,
    stale,
    members: ['representative', 'other', 'excluded'].map((id) => ({
      asset: {
        ...golden.assetDetail,
        id,
        filename: `${id}.jpg`,
        relativePath: `trip/${id}.jpg`,
      } as AssetSummary,
      distance: null,
      excluded: allExcluded || id === 'excluded' || (id === 'other' && otherExcluded),
    })),
  };
  invoke.mockImplementation(async (name: string) => {
    if (name === 'query_duplicate_groups') return { items: [group], total: 1, nextCursor: null };
    if (name === 'set_group_representative') return { updated: true };
    if (name === 'apply_review_batch')
      return { changeBatchId: 'batch', affectedCount: 1, unchangedCount: 0, conflicts: [] };
    throw new Error(name);
  });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/duplicates', component: DuplicatesPage }],
  });
  await router.push('/duplicates');
  wrapper = mount(DuplicatesPage, {
    global: {
      plugins: [router],
      stubs: { Icon: true, EmptyState: true, MediaThumbnail: true, BurstGroups: true },
    },
  });
  await flushPromises();
  return wrapper.findAll('button').find((button) => button.text() === '标记当前显示的其余照片淘汰')!;
}

it.each([
  { excluded: true, stale: false },
  { excluded: false, stale: true },
])('does not offer an empty or stale group rejection ($excluded/$stale)', async ({ excluded, stale }) => {
  const button = await showGroup(excluded, stale);
  expect(button.attributes('disabled')).toBeDefined();
  await button.trigger('click');
  expect(confirm).not.toHaveBeenCalled();
  expect(invoke.mock.calls.filter(([name]) => name === 'apply_review_batch')).toHaveLength(0);
});

it('previews and marks only the displayed eligible members after explicit confirmation', async () => {
  const button = await showGroup(false);
  expect(button.attributes('disabled')).toBeUndefined();
  await button.trigger('click');
  await flushPromises();
  expect(confirm.mock.calls[0]![0]).toBe('将以下 1 张标记淘汰，仅修改资料库：\ntrip/other.jpg');
  const writes = invoke.mock.calls.filter(([name]) => name === 'apply_review_batch');
  expect(writes).toHaveLength(1);
  expect(writes[0]![1]).toMatchObject({
    target: { kind: 'explicit', assetIds: ['other'] },
    patch: { pickState: 'reject' },
  });
});

it('does not open a copy wizard when every displayed candidate is excluded', async () => {
  await showGroup(false, false, true);
  const button = wrapper!.findAll('button').find((button) => button.text() === '复制当前显示文件')!;
  expect(button.attributes('disabled')).toBeDefined();
  await button.trigger('click');
  expect(useApp().exportOpen).toBe(false);
});

it('refreshes a stale exact group when recomputation finishes between job polls', async () => {
  await showGroup(false, true);
  expect(wrapper!.text()).toContain('结果过期');
  invoke.mockResolvedValueOnce({ items: [], total: 0, nextCursor: null });
  useApp().jobs = [
    {
      id: 'recomputed',
      type: 'HASH_EXACT',
      operation: 'recomputeExact',
      status: 'completed',
      progressDone: 1,
      progressTotal: 1,
      failureCount: 0,
      errorSummary: null,
      createdAt: 1,
      updatedAt: 2,
    },
  ];
  await flushPromises();
  expect(wrapper!.text()).toContain('0 个精确重复组');
  expect(wrapper!.text()).not.toContain('结果过期');
});
