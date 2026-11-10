import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import DuplicatesPage from '../pages/DuplicatesPage.vue';
import type { AssetSummary, DuplicateGroup } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, isDemo: false, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({
  ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
  ElMessageBox: { confirm: vi.fn() },
}));
let wrapper: VueWrapper | undefined;
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
});

async function showGroup(excludedIndices: number[] = [], kind: 'exact' | 'similar' = 'exact') {
  const excluded = new Set(excludedIndices);
  const members = Array.from({ length: 125 }, (_, index) => ({
    asset: {
      ...golden.assetDetail,
      id: `asset-${String(index).padStart(3, '0')}`,
      filename: `photo-${index}.jpg`,
      byteSize: 1024,
    } as AssetSummary,
    distance: null,
    excluded: excluded.has(index),
  }));
  const group: DuplicateGroup = {
    id: 'group',
    kind,
    algorithm: kind === 'exact' ? 'SHA-256' : 'dHash-64',
    algorithmVersion: '1',
    memberCount: members.filter((m) => !m.excluded).length,
    totalMembers: members.length,
    representativeAssetId: members.find((m) => !m.excluded)?.asset.id ?? null,
    stale: false,
    members: members.slice(0, 12),
  };
  invoke.mockImplementation(async (command: string, request: { cursor?: string }) => {
    if (command === 'query_duplicate_groups') return { items: [group], total: 1, nextCursor: null };
    if (command === 'query_group_members') {
      const offset = members.findIndex((m) => m.asset.id === request.cursor) + 1;
      return { items: members.slice(offset, offset + 50), total: 125, nextCursor: null };
    }
    throw new Error(command);
  });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/duplicates', component: DuplicatesPage }],
  });
  await router.push(`/duplicates?kind=${kind}`);
  wrapper = mount(DuplicatesPage, {
    global: {
      plugins: [router],
      stubs: { Icon: true, EmptyState: true, MediaThumbnail: true, BurstGroups: true },
    },
  });
  await flushPromises();
  return wrapper;
}
function header() {
  return wrapper!.get('.duplicate-group header').text().replace(/\s+/g, ' ');
}

it('estimates the entire exact group from its effective count before later members are loaded', async () => {
  await showGroup();
  expect(wrapper!.findAll('.duplicate-members article')).toHaveLength(12);
  expect(header()).toContain('125 个有效成员');
  expect(header()).toContain('全组理论冗余约 124 KB，仅估算');
});
it('keeps the same full-group estimate through 12,62,112,125 member pagination', async () => {
  await showGroup();
  const initial = header().match(/(?:全组)?理论冗余约.*?仅估算/)![0];
  for (const count of [62, 112, 125]) {
    await wrapper!.get('button[aria-label^="载入第 1 组更多照片"]').trigger('click');
    await flushPromises();
    expect(wrapper!.findAll('.duplicate-members article')).toHaveLength(count);
    expect(header()).toContain(initial);
  }
  expect(header()).toContain('124 KB');
  expect(wrapper!.find('button[aria-label^="载入第 1 组更多照片"]').exists()).toBe(false);
});
it('excludes both loaded and unloaded decisions from the redundant member count', async () => {
  await showGroup([3, 120]);
  expect(header()).toContain('123 个有效成员');
  expect(header()).toContain('122 KB');
});
it('uses the shared exact-byte size even when every displayed member is excluded', async () => {
  await showGroup(Array.from({ length: 12 }, (_, i) => i));
  expect(header()).toContain('113 个有效成员');
  expect(header()).toContain('112 KB');
});
it.each([0, 1])('reports zero redundant bytes with only %i effective members', async (effective) => {
  await showGroup(Array.from({ length: 125 - effective }, (_, i) => i));
  expect(header()).toContain(`${effective} 个有效成员`);
  expect(header()).toContain('理论冗余约 0 B，仅估算');
});
it('does not infer byte redundancy for merely visually similar members', async () => {
  await showGroup([], 'similar');
  expect(header()).not.toContain('理论冗余');
});
