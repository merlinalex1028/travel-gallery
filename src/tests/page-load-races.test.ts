import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { flushPromises, mount } from '@vue/test-utils';
import DuplicatesPage from '../pages/DuplicatesPage.vue';
import MissingPage from '../pages/MissingPage.vue';
import { useApp } from '../stores/app';
import { files as fileLabels } from '../services/format';
import type { AssetSummary, DuplicateGroup, SourceRoot } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({
  api: invoke,
  errorMessage: (error: { message?: string }) => error.message ?? 'request failed',
  isDemo: false,
}));
vi.mock('element-plus', () => ({
  ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() },
  ElMessageBox: { confirm: vi.fn().mockResolvedValue(false) },
}));
function pending<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function asset(n: number, extra: Partial<AssetSummary> = {}): AssetSummary {
  return {
    ...(structuredClone(golden.assetDetail) as AssetSummary),
    id: `asset-${String(n).padStart(3, '0')}`,
    sourceId: 'one',
    filename: `owned-${n}.jpg`,
    relativePath: `owned-${n}.jpg`,
    ...extra,
  };
}
function group(kind: DuplicateGroup['kind'] = 'exact', count = 125): DuplicateGroup {
  return {
    id: `${kind}-group`,
    kind,
    algorithm: kind === 'exact' ? 'SHA-256' : 'dHash-64',
    algorithmVersion: '1',
    memberCount: count,
    totalMembers: count,
    representativeAssetId: asset(0).id,
    stale: false,
    members: Array.from({ length: Math.min(12, count) }, (_, n) => ({
      asset: asset(n),
      distance: kind === 'exact' ? null : 0,
      excluded: false,
    })),
  };
}
function source(id = 'one', absolutePath = '/owned/one'): SourceRoot {
  return {
    id,
    absolutePath,
    displayName: '同名来源',
    volumeId: null,
    isEnabled: true,
    isOnline: true,
    lastScanAt: null,
    totalAssets: 125,
    offlineAssets: 0,
  };
}
async function setup(page: 'duplicates' | 'missing') {
  const pinia = createPinia();
  setActivePinia(pinia);
  useApp().sources = [source(), source('two', '/owned/two')];
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/duplicates', '/missing', '/sources', '/viewer/:assetId'].map((path) => ({
      path,
      component: { render: () => null },
    })),
  });
  await router.push(`/${page}`);
  await router.isReady();
  const wrapper = mount(page === 'duplicates' ? DuplicatesPage : MissingPage, {
    global: { plugins: [pinia, router], stubs: { MediaThumbnail: true, SourceCard: true } },
  });
  await flushPromises();
  return wrapper;
}
beforeEach(() => {
  invoke.mockReset();
});

it('loads all 125 unique group members after rapid clicks without prematurely hiding pagination', async () => {
  const waiting: ReturnType<typeof pending<unknown>>[] = [];
  invoke.mockImplementation(async (command: string) => {
    if (command === 'query_duplicate_groups') return { items: [group()], total: 1, nextCursor: null };
    if (command === 'query_group_members') {
      const next = pending<unknown>();
      waiting.push(next);
      return next.promise;
    }
    throw new Error(command);
  });
  const wrapper = await setup('duplicates');
  try {
    const more = () =>
      wrapper.findAll('button').find((button) => button.text().includes('载入更多组内照片'))!;
    more().element.click();
    more().element.click();
    await flushPromises();
    expect(waiting).toHaveLength(1);
    expect(
      wrapper
        .findAll('button')
        .find((button) => button.text().includes('正在载入组内照片'))!
        .attributes('disabled'),
    ).toBeDefined();
    for (const [index, start, count] of [
      [0, 12, 50],
      [1, 62, 50],
      [2, 112, 13],
    ]) {
      if (index) {
        await more().trigger('click');
        await flushPromises();
      }
      waiting[index]!.resolve({
        items: Array.from({ length: count }, (_, n) => ({
          asset: asset(start + n),
          excluded: false,
          distance: null,
        })),
        total: 125,
        nextCursor: null,
      });
      await flushPromises();
    }
    const captions = wrapper.findAll('.duplicate-caption strong').map((caption) => caption.text());
    expect(captions).toHaveLength(125);
    expect(new Set(captions).size).toBe(125);
    expect(more()).toBeUndefined();
    expect(
      invoke.mock.calls
        .filter(([command]) => command === 'query_group_members')
        .map(([, request]) => request.cursor),
    ).toEqual([asset(11).id, asset(61).id, asset(111).id]);
  } finally {
    wrapper.unmount();
  }
});
it('keeps a member-page error visible and retries without losing loaded members', async () => {
  let fail = true;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'query_duplicate_groups')
      return { items: [group('exact', 13)], total: 1, nextCursor: null };
    if (fail) throw new Error('cannot read members');
    return { items: [{ asset: asset(12), excluded: false, distance: null }], total: 13, nextCursor: null };
  });
  const wrapper = await setup('duplicates');
  try {
    await wrapper
      .findAll('button')
      .find((button) => button.text().includes('载入更多组内照片'))!
      .trigger('click');
    await flushPromises();
    expect(wrapper.get('[role="alert"]').text()).toContain('cannot read members');
    expect(wrapper.findAll('.duplicate-caption strong')).toHaveLength(12);
    fail = false;
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '重试载入')!
      .trigger('click');
    await flushPromises();
    expect(wrapper.findAll('.duplicate-caption strong')).toHaveLength(13);
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  } finally {
    wrapper.unmount();
  }
});
it('ignores a late member response after changing duplicate kind', async () => {
  const old = pending<unknown>();
  invoke.mockImplementation(async (command: string, request: { kind: DuplicateGroup['kind'] }) => {
    if (command === 'query_duplicate_groups')
      return { items: [group(request.kind, request.kind === 'exact' ? 125 : 2)], total: 1, nextCursor: null };
    return old.promise;
  });
  const wrapper = await setup('duplicates');
  try {
    await wrapper
      .findAll('button')
      .find((button) => button.text().includes('载入更多组内照片'))!
      .trigger('click');
    await wrapper
      .findAll('.tab-line button')
      .find((button) => button.text().includes('视觉相似'))!
      .trigger('click');
    await flushPromises();
    old.resolve({
      items: [{ asset: asset(99), excluded: false, distance: null }],
      total: 125,
      nextCursor: null,
    });
    await flushPromises();
    expect(wrapper.findAll('.duplicate-caption strong')).toHaveLength(2);
    expect(wrapper.text()).not.toContain('owned-99.jpg');
  } finally {
    wrapper.unmount();
  }
});
it('allows refreshed members to load while an obsolete request remains pending', async () => {
  const old = pending<unknown>(),
    current = pending<unknown>();
  let memberRequests = 0;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'query_duplicate_groups')
      return { items: [group('exact', 13)], total: 1, nextCursor: null };
    return ++memberRequests === 1 ? old.promise : current.promise;
  });
  const wrapper = await setup('duplicates');
  try {
    const more = () =>
      wrapper.findAll('button').find((button) => button.text().includes('载入更多组内照片'))!;
    await more().trigger('click');
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '刷新')!
      .trigger('click');
    await flushPromises();
    expect(more().attributes('disabled')).toBeUndefined();
    await more().trigger('click');
    await flushPromises();
    expect(memberRequests).toBe(2);
    old.resolve({
      items: [{ asset: asset(99), excluded: false, distance: null }],
      total: 125,
      nextCursor: null,
    });
    await flushPromises();
    expect(wrapper.text()).not.toContain('owned-99.jpg');
    const busy = wrapper.findAll('button').find((button) => button.text().includes('正在载入组内照片'))!;
    expect(busy.attributes('disabled')).toBeDefined();
    current.resolve({
      items: [{ asset: asset(12), excluded: false, distance: null }],
      total: 13,
      nextCursor: null,
    });
    await flushPromises();
    expect(wrapper.findAll('.duplicate-caption strong')).toHaveLength(13);
  } finally {
    wrapper.unmount();
  }
});
it('clears old-kind actions when a new-kind query fails and recovers via the visible retry', async () => {
  let fail = true;
  invoke.mockImplementation(async (_command: string, request: { kind: DuplicateGroup['kind'] }) => {
    if (request.kind === 'similar' && fail) throw new Error('cannot query similar groups');
    return { items: [group(request.kind, 2)], total: 1, nextCursor: null };
  });
  const wrapper = await setup('duplicates');
  try {
    await wrapper
      .findAll('.tab-line button')
      .find((button) => button.text().includes('视觉相似'))!
      .trigger('click');
    await flushPromises();
    expect(wrapper.find('.duplicate-group').exists()).toBe(false);
    expect(wrapper.get('[role="alert"]').text()).toContain('cannot query similar groups');
    fail = false;
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '重试')!
      .trigger('click');
    await flushPromises();
    expect(wrapper.findAll('.duplicate-group')).toHaveLength(1);
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  } finally {
    wrapper.unmount();
  }
});
it('shows a next-group-page failure persistently while retaining the valid current-kind rows', async () => {
  let fail = false;
  invoke.mockImplementation(async (_command: string, request: { cursor: string | null }) => {
    if (request.cursor && fail) throw new Error('next group page failed');
    return { items: [group('exact', 2)], total: 21, nextCursor: '20' };
  });
  const wrapper = await setup('duplicates');
  try {
    fail = true;
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '加载更多分组')!
      .trigger('click');
    await flushPromises();
    expect(wrapper.findAll('.duplicate-group')).toHaveLength(1);
    expect(wrapper.get('[role="alert"]').text()).toContain('next group page failed');
    fail = false;
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '重新加载分组')!
      .trigger('click');
    await flushPromises();
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  } finally {
    wrapper.unmount();
  }
});
it('keeps the new file-state rows and cursor when an older response arrives last', async () => {
  const old = pending<unknown>(),
    current = pending<unknown>();
  invoke.mockImplementation(async (_command: string, request: { filter: { fileStates: string[] } }) =>
    request.filter.fileStates[0] === 'missing' ? old.promise : current.promise,
  );
  const wrapper = await setup('missing');
  try {
    await wrapper
      .findAll('.segmented button')
      .find((button) => button.text() === fileLabels.offline)!
      .trigger('click');
    current.resolve({ items: [asset(1, { fileState: 'offline' })], total: 1, nextCursor: null });
    await flushPromises();
    old.resolve({ items: [asset(99, { fileState: 'missing' })], total: 500, nextCursor: 'wrong' });
    await flushPromises();
    expect(wrapper.text()).toContain('owned-1.jpg');
    expect(wrapper.text()).not.toContain('owned-99.jpg');
    expect(
      wrapper
        .findAll('.segmented button')
        .find((button) => button.attributes('aria-pressed') === 'true')!
        .text(),
    ).toBe(fileLabels.offline);
    expect(wrapper.findAll('button').some((button) => button.text().includes('载入更多（'))).toBe(false);
  } finally {
    wrapper.unmount();
  }
});
it('distinguishes a failed file-state query from a valid empty result and allows retry', async () => {
  let fail = true;
  invoke.mockImplementation(async () => {
    if (fail) throw new Error('file-state query failed');
    return { items: [], total: 0, nextCursor: null };
  });
  const wrapper = await setup('missing');
  try {
    expect(wrapper.get('[role="alert"]').text()).toContain('file-state query failed');
    expect(wrapper.text()).toContain('文件状态暂不可用');
    expect(wrapper.text()).not.toContain(`没有${fileLabels.missing}的文件`);
    fail = false;
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '重试检查')!
      .trigger('click');
    await flushPromises();
    expect(wrapper.find('[role="alert"]').exists()).toBe(false);
    expect(wrapper.text()).toContain(`没有${fileLabels.missing}的文件`);
  } finally {
    wrapper.unmount();
  }
});
it('prevents duplicate file-state page requests while loading', async () => {
  const next = pending<unknown>();
  invoke.mockImplementation(async (_command: string, request: { cursor: string | null }) =>
    request.cursor
      ? next.promise
      : { items: [asset(0, { fileState: 'missing' })], total: 2, nextCursor: 'next' },
  );
  const wrapper = await setup('missing');
  try {
    const button = wrapper.findAll('button').find((button) => button.text().includes('载入更多（'))!;
    button.element.click();
    button.element.click();
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(2);
    next.resolve({ items: [asset(1, { fileState: 'missing' })], total: 2, nextCursor: null });
    await flushPromises();
    expect(wrapper.findAll('.table-row')).toHaveLength(2);
  } finally {
    wrapper.unmount();
  }
});
it('separates identical relative filenames by source root and shows loaded counts', async () => {
  invoke.mockResolvedValue({
    items: [
      asset(0, { filename: 'same.jpg', relativePath: 'same.jpg', sourceId: 'one' }),
      asset(1, { filename: 'same.jpg', relativePath: 'same.jpg', sourceId: 'two' }),
    ],
    total: 2,
    nextCursor: null,
  });
  const wrapper = await setup('missing');
  try {
    const sections = wrapper.findAll('.status-source-group');
    expect(sections).toHaveLength(2);
    expect(sections[0]!.text()).toContain('/owned/one');
    expect(sections[1]!.text()).toContain('/owned/two');
    expect(wrapper.text()).toContain('当前状态 2 项，已加载 2 项');
  } finally {
    wrapper.unmount();
  }
});
it('shows duplicate-member source roots, ratings and confirmed pairing without guessing from a RAW extension', async () => {
  const result = group('exact', 3);
  result.members = [
    {
      asset: asset(0, {
        linkedMediaCount: 1,
        linkedMediaType: 'live_photo',
        review: { ...asset(0).review, rating: 4 },
      }),
      excluded: false,
      distance: null,
    },
    {
      asset: asset(1, { sourceId: 'two', linkedMediaCount: 1, linkedMediaType: 'raw_jpeg' }),
      excluded: false,
      distance: null,
    },
    {
      asset: asset(2, {
        extension: 'cr2',
        linkedMediaCount: 0,
        linkedMediaType: null,
        review: { ...asset(2).review, rating: 0 },
      }),
      excluded: false,
      distance: null,
    },
  ];
  invoke.mockResolvedValue({ items: [result], total: 1, nextCursor: null });
  const wrapper = await setup('duplicates');
  try {
    const cards = wrapper.findAll('.duplicate-members article');
    expect(cards[0]!.text()).toContain('/owned/one');
    expect(cards[1]!.text()).toContain('/owned/two');
    expect(cards[0]!.find('.duplicate-review').text()).toContain('4 星');
    expect(cards[0]!.find('.duplicate-link').text()).toBe('Live Photo · 2 个源文件');
    expect(cards[1]!.find('.duplicate-link').text()).toBe('RAW + JPEG · 2 个源文件');
    expect(cards[2]!.find('.duplicate-review').text()).toBe('CR2 · 未评分');
    expect(cards[2]!.find('.duplicate-link').text()).toBe('未确认配套');
  } finally {
    wrapper.unmount();
  }
});
