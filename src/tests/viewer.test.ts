import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { defineComponent, h } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { flushPromises, mount } from '@vue/test-utils';
import { useViewer } from '../composables/viewer';
import { useLibrary } from '../stores/library';
import { useApp } from '../stores/app';
import { stageGeometry } from '../services/stage';
import ViewerPage from '../pages/ViewerPage.vue';
import type { AssetDetail, AssetFilter, CullSession } from '../services/contracts';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }));
function asset(id: string): AssetDetail {
  return {
    id,
    sourceId: 's',
    relativePath: `${id}.jpg`,
    filename: `${id}.jpg`,
    extension: 'jpg',
    mediaKind: 'image',
    fileState: 'available',
    byteSize: 1,
    captureUtcMs: null,
    captureLocal: null,
    width: 6000,
    height: 4000,
    durationMs: null,
    cameraModel: null,
    thumbnailState: 'ready',
    review: { pickState: 'unreviewed', rating: 0, isFavorite: false, note: '', revision: 0, updatedAt: 0 },
    absolutePath: `/sample/${id}.jpg`,
    cameraMake: null,
    codec: null,
    gpsLat: null,
    gpsLon: null,
    orientation: 1,
    captureTzMin: null,
    captureTimeSource: null,
    metadata: {},
    tagIds: [],
    preview: {
      canThumbnail: true,
      canScreenPreview: true,
      canOriginalPreview: true,
      canPlayInline: false,
      canOpenSystem: true,
    },
  };
}
let records: AssetDetail[],
  contexts: Map<string, AssetFilter>,
  saved: CullSession | null,
  writes: number,
  failWrite: boolean,
  failQuery: boolean;
beforeEach(() => {
  setActivePinia(createPinia());
  records = ['a', 'b', 'c'].map(asset);
  contexts = new Map([['initial', { selectionSnapshotId: 'scope', pickStates: ['unreviewed'] }]]);
  saved = null;
  writes = 0;
  failWrite = false;
  failQuery = false;
  invoke.mockReset();
  const matching = (f: AssetFilter) =>
    records.filter(
      (a) =>
        (f.selectionSnapshotId ? ['a', 'b'].includes(a.id) : a.id !== 'c') &&
        (!f.pickStates?.length || f.pickStates.includes(a.review.pickState)),
    );
  invoke.mockImplementation(async (name: string, r: any) => {
    if (name === 'get_asset') return structuredClone(records.find((a) => a.id === r.assetId));
    if (name === 'query_assets') {
      if (failQuery) throw new Error('snapshot expired');
      const filter = r.filter as AssetFilter,
        items = matching(filter);
      const fp = JSON.stringify(filter);
      contexts.set(fp, filter);
      return { items: structuredClone(items), total: items.length, nextCursor: null, queryFingerprint: fp };
    }
    if (name === 'get_asset_neighbors') {
      const items = matching(contexts.get(r.queryFingerprint) ?? {});
      const index = items.findIndex((a) => a.id === r.assetId);
      const anchor = records.find((a) => a.id === r.assetId);
      return {
        previousId: items.filter((a) => a.id < r.assetId).at(-1)?.id ?? null,
        nextId: items.find((a) => a.id > r.assetId)?.id ?? null,
        items: structuredClone(index < 0 && anchor ? [...items, anchor] : items),
        anchorMatches: index >= 0,
        total: items.length,
        position: index < 0 ? null : index + 1,
      };
    }
    if (name === 'save_session') {
      saved = structuredClone(r);
      return { updated: true };
    }
    if (name === 'get_session') return saved;
    if (name === 'get_cull_progress') {
      const items = matching({ ...r.filter, pickStates: undefined });
      const count = {
        total: items.length,
        reviewed: items.filter((a) => a.review.pickState !== 'unreviewed').length,
      };
      return {
        scope: count,
        day: { label: '2026-06-15', ...count },
        folder: { label: '来源根目录', ...count },
      };
    }
    if (name === 'resolve_preview') return { ready: false };
    if (name === 'apply_review_batch') {
      if (failWrite) throw new Error('disk failure');
      writes++;
      const a = records.find((a) => a.id === r.target.assetIds[0])!;
      Object.assign(a.review, r.patch);
      a.review.revision++;
      return { changeBatchId: `batch-${writes}`, affectedCount: 1, unchangedCount: 0, conflicts: [] };
    }
    throw new Error(`Unexpected command ${name}`);
  });
});
async function setup(resume = false, normalViewer = false, normalFilter: AssetFilter = {}) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/cull', '/filter', '/library', '/viewer/:assetId'].map((path) => ({
      path,
      component: { render: () => null },
    })),
  });
  await router.push(normalViewer ? '/viewer/a' : '/cull');
  await router.isReady();
  const lib = useLibrary();
  lib.total = 2;
  lib.session = {
    filter: { selectionSnapshotId: 'scope', pickStates: ['unreviewed'] },
    scopeFilter: { selectionSnapshotId: 'scope' },
    sort: { field: 'filename', direction: 'asc' },
    queryFingerprint: 'initial',
    anchorAssetId: 'a',
    returnPath: '/filter',
    returnFilter: { ratingMin: 4 },
    returnSort: { field: 'rating', direction: 'desc' },
    returnScrollTop: 1234,
    total: 2,
  };
  if (normalViewer) {
    lib.session = { ...lib.session, filter: normalFilter, scopeFilter: undefined };
    contexts.set('initial', normalFilter);
  }
  if (resume) {
    saved = structuredClone(lib.session);
    lib.session = null;
  }
  let v!: ReturnType<typeof useViewer>;
  const wrapper = mount(
    defineComponent({
      setup() {
        v = useViewer();
        return () => null;
      },
    }),
    { global: { plugins: [router] } },
  );
  await flushPromises();
  return { v, lib, wrapper, router };
}
describe('normal viewer query counters', () => {
  it('uses the same live query position and total as neighbors after an earlier asset is indexed', async () => {
    const { v, lib, wrapper } = await setup(false, true);
    try {
      records.unshift(asset('0'));
      await v.jump('b');
      expect(v.filmstrip.value.map((a) => a.id)).toEqual(['0', 'a', 'b']);
      expect(v.position.value).toBe(3);
      expect(v.initialTotal.value).toBe(3);
      expect(lib.session).toMatchObject({ position: 3, total: 3, filter: {} });
    } finally {
      wrapper.unmount();
    }
  });
  it('updates both counters when a preceding member leaves the query before reloading the anchor', async () => {
    const { v, lib, wrapper } = await setup(false, true);
    try {
      await v.jump('b');
      records = records.filter((a) => a.id !== 'a');
      await v.undo();
      expect(v.asset.value?.id).toBe('b');
      expect(v.position.value).toBe(1);
      expect(v.initialTotal.value).toBe(1);
      expect(lib.session).toMatchObject({ position: 1, total: 1 });
    } finally {
      wrapper.unmount();
    }
  });
  it('keeps a frozen cull total while its remaining-query count shrinks', async () => {
    const { v, lib, wrapper } = await setup();
    await v.mark({ pickState: 'keep' });
    expect(v.asset.value?.id).toBe('b');
    expect(v.filmstrip.value.map((a) => a.id)).toEqual(['b']);
    expect(v.initialTotal.value).toBe(2);
    expect(lib.session?.total).toBe(2);
    expect(v.processed.value).toBe(1);
    wrapper.unmount();
  });
  it('marks the current asset outside its filter after a committed review instead of showing a false rank', async () => {
    const { v, lib, wrapper } = await setup(false, true, { pickStates: ['unreviewed'] });
    try {
      await v.jump('b');
      await v.mark({ pickState: 'keep' });
      expect(v.asset.value?.id).toBe('b');
      expect(v.matchesQuery.value).toBe(false);
      expect(v.initialTotal.value).toBe(1);
      expect(lib.session?.filter).toEqual({ pickStates: ['unreviewed'] });
      await v.navigate(-1);
      expect(v.asset.value?.id).toBe('a');
      expect(v.matchesQuery.value).toBe(true);
      expect(v.position.value).toBe(1);
      expect(v.initialTotal.value).toBe(1);
    } finally {
      wrapper.unmount();
    }
  });
});
describe('cull session', () => {
  it('updates the position when selecting another filmstrip item', async () => {
    const { v, wrapper } = await setup();
    await v.jump('b');
    expect(v.asset.value?.id).toBe('b');
    expect(v.position.value).toBe(2);
    await v.jump('a');
    expect(v.asset.value?.id).toBe('a');
    expect(v.position.value).toBe(1);
    await v.jump('outside-scope');
    expect(v.position.value).toBe(1);
    wrapper.unmount();
  });
  it('restores the anchor position when undoing after advancing', async () => {
    const { v, wrapper } = await setup();
    await v.mark({ pickState: 'keep' });
    expect(v.asset.value?.id).toBe('b');
    expect(v.position.value).toBe(2);
    const implementation = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, request: any) => {
      if (command === 'undo_review_batch') {
        records[0]!.review.pickState = 'unreviewed';
        return { restoredCount: 1, skippedCount: 0 };
      }
      return implementation(command, request);
    });
    await v.undo();
    expect(v.asset.value?.id).toBe('a');
    expect(v.position.value).toBe(1);
    expect(v.processed.value).toBe(0);
    wrapper.unmount();
  });
  it('applies 500 rapid favorite shortcuts to the committed current asset', async () => {
    useApp().settings.autoAdvance = false;
    const { v, wrapper } = await setup();
    for (let n = 0; n < 500; n++)
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', bubbles: true }));
    await vi.waitFor(() => expect(writes).toBe(500), { timeout: 4000 });
    await flushPromises();
    expect(records[0]!.review.isFavorite).toBe(false);
    expect(v.asset.value?.review.revision).toBe(500);
    expect(v.asset.value?.id).toBe('a');
    wrapper.unmount();
  });
  it('moves only after commit, completes a frozen scope, and restores the library context', async () => {
    const { v, lib, wrapper, router } = await setup();
    await v.mark({ pickState: 'keep' });
    expect(v.asset.value?.id).toBe('b');
    await v.mark({ pickState: 'reject' });
    expect(v.complete.value).toBe(true);
    expect(v.summary.value).toEqual({ keep: 1, reject: 1, maybe: 0, unreviewed: 0 });
    expect(v.progress.value?.scope).toEqual({ total: 2, reviewed: 2 });
    await v.changeScope('all');
    expect(v.initialTotal.value).toBe(2);
    expect(lib.filter.selectionSnapshotId).toBe('scope');
    expect(lib.items.map((a) => a.id)).toEqual(['a', 'b']);
    v.close();
    await flushPromises();
    expect(router.currentRoute.value.path).toBe('/filter');
    expect(lib.filter).toEqual({ ratingMin: 4 });
    expect(lib.scrollTop).toBe(1234);
    expect(lib.sort.field).toBe('rating');
    wrapper.unmount();
  });
  it('shows an expired restored snapshot as an error instead of declaring completion', async () => {
    failQuery = true;
    const { v, wrapper } = await setup(true);
    expect(v.complete.value).toBe(false);
    expect(v.error.value).toBe('snapshot expired');
    expect(v.asset.value).toBe(null);
    wrapper.unmount();
  });
  it('keeps the current asset and confirmed state after a failed write', async () => {
    const { v, wrapper } = await setup();
    failWrite = true;
    await v.mark({ pickState: 'keep' });
    expect(v.asset.value?.id).toBe('a');
    expect(v.asset.value?.review.pickState).toBe('unreviewed');
    expect(v.error.value).toBe('disk failure');
    expect(v.saved.value).toBe(false);
    wrapper.unmount();
  });
  it('retries a failed favorite with the same intent, mutation ID and revision, then advances only once', async () => {
    const { v, lib, wrapper } = await setup();
    try {
      failWrite = true;
      await v.mark(() => ({ isFavorite: !v.asset.value?.review.isFavorite }));
      expect(v.canRetry.value).toBe(true);
      expect(v.busy.value).toBe(false);
      expect(v.asset.value?.id).toBe('a');
      const first = invoke.mock.calls.find(([command]) => command === 'apply_review_batch')![1];
      expect(first.expectedRevisions).toEqual({ a: 0 });
      failWrite = false;
      await Promise.all([v.retryReview(), v.retryReview()]);
      const requests = invoke.mock.calls
        .filter(([command]) => command === 'apply_review_batch')
        .map(([, r]) => r);
      expect(requests).toHaveLength(2);
      expect(requests[1]).toEqual(first);
      expect(records[0]!.review.isFavorite).toBe(true);
      expect(records[0]!.review.revision).toBe(1);
      expect(v.asset.value?.id).toBe('b');
      expect(v.position.value).toBe(2);
      expect(v.canRetry.value).toBe(false);
      expect(v.saved.value).toBe(true);
      expect(v.error.value).toBe('');
      expect(lib.history).toEqual(['batch-1']);
    } finally {
      wrapper.unmount();
    }
  });
  it('replays a lost write acknowledgment without producing a second transaction', async () => {
    const { v, lib, wrapper } = await setup();
    try {
      const implementation = invoke.getMockImplementation()!;
      let receipt: unknown, payload: unknown;
      invoke.mockImplementation(async (command: string, request: any) => {
        if (command !== 'apply_review_batch') return implementation(command, request);
        if (receipt) {
          expect(request).toEqual(payload);
          return receipt;
        }
        payload = structuredClone(request);
        receipt = await implementation(command, request);
        throw new Error('acknowledgment lost');
      });
      await v.mark({ pickState: 'keep' });
      expect(records[0]!.review.pickState).toBe('keep');
      expect(v.asset.value?.review.pickState).toBe('unreviewed');
      expect(v.asset.value?.id).toBe('a');
      expect(v.canRetry.value).toBe(true);
      await v.retryReview();
      expect(writes).toBe(1);
      expect(records[0]!.review.revision).toBe(1);
      expect(lib.history).toEqual(['batch-1']);
      expect(v.processed.value).toBe(1);
      expect(v.asset.value?.id).toBe('b');
    } finally {
      wrapper.unmount();
    }
  });
  it('retries only refreshing after a confirmed commit and counts the transaction once', async () => {
    const { v, lib, wrapper } = await setup();
    try {
      useApp().settings.autoAdvance = false;
      const implementation = invoke.getMockImplementation()!;
      let failDetails = true;
      invoke.mockImplementation(async (command: string, request: any) => {
        if (command === 'get_asset' && failDetails) throw new Error('detail refresh failed');
        if (command === 'undo_review_batch') {
          records[0]!.review.pickState = 'unreviewed';
          return { restoredCount: 1, skippedCount: 0 };
        }
        return implementation(command, request);
      });
      await v.mark({ pickState: 'keep' });
      expect(v.saved.value).toBe(true);
      expect(v.canRetry.value).toBe(true);
      expect(v.processed.value).toBe(1);
      failDetails = false;
      await v.retryReview();
      expect(writes).toBe(1);
      expect(v.processed.value).toBe(1);
      expect(lib.history).toEqual(['batch-1']);
      await v.undo();
      expect(v.processed.value).toBe(0);
      expect(v.asset.value?.review.pickState).toBe('unreviewed');
    } finally {
      wrapper.unmount();
    }
  });
  it('refreshes a revision conflict without silently overwriting the newer review', async () => {
    const { v, wrapper } = await setup();
    try {
      const implementation = invoke.getMockImplementation()!;
      let conflict = true;
      invoke.mockImplementation(async (command: string, request: any) => {
        if (command === 'apply_review_batch' && conflict)
          throw { code: 'DB_CONFLICT', message: '照片已被其他操作更新，本批未写入' };
        return implementation(command, request);
      });
      await v.mark({ pickState: 'keep' });
      records[0]!.review.pickState = 'maybe';
      records[0]!.review.revision = 2;
      expect(v.retryLabel.value).toBe('刷新照片');
      await v.retryReview();
      expect(writes).toBe(0);
      expect(v.asset.value?.review.pickState).toBe('maybe');
      expect(v.saved.value).toBe(true);
      expect(v.canRetry.value).toBe(false);
      conflict = false;
      await v.mark({ pickState: 'keep' });
      const last = invoke.mock.calls.filter(([command]) => command === 'apply_review_batch').at(-1)![1];
      expect(last.expectedRevisions).toEqual({ a: 2 });
      expect(writes).toBe(1);
    } finally {
      wrapper.unmount();
    }
  });
  it('discards a failed action when explicitly navigating away instead of retrying on a different asset', async () => {
    const { v, wrapper } = await setup();
    try {
      failWrite = true;
      await v.mark({ pickState: 'keep' });
      await v.navigate(1);
      expect(v.asset.value?.id).toBe('b');
      expect(v.canRetry.value).toBe(false);
      failWrite = false;
      await v.retryReview();
      expect(writes).toBe(0);
      expect(records[1]!.review.pickState).toBe('unreviewed');
    } finally {
      wrapper.unmount();
    }
  });
  it('keeps a conflict refresh available after a read failure and never resubmits the stale write', async () => {
    const { v, wrapper } = await setup();
    try {
      const implementation = invoke.getMockImplementation()!;
      let failRead = false;
      invoke.mockImplementation(async (command: string, request: any) => {
        if (command === 'apply_review_batch') throw { code: 'DB_CONFLICT', message: '记录已有变化' };
        if (command === 'get_asset' && failRead) throw new Error('cannot read');
        return implementation(command, request);
      });
      await v.mark({ rating: 4 });
      failRead = true;
      await v.retryReview();
      expect(v.canRetry.value).toBe(true);
      expect(v.retryLabel.value).toBe('刷新照片');
      failRead = false;
      await v.retryReview();
      expect(v.canRetry.value).toBe(false);
      expect(v.saved.value).toBe(true);
      expect(invoke.mock.calls.filter(([command]) => command === 'apply_review_batch')).toHaveLength(1);
    } finally {
      wrapper.unmount();
    }
  });
  it('renders an explicit retry and an unconfirmed status, then confirms and advances after recovery', async () => {
    const harness = await setup();
    harness.wrapper.unmount();
    await harness.router.push('/cull');
    await harness.router.isReady();
    const wrapper = mount(ViewerPage, {
      global: {
        plugins: [harness.router],
        stubs: {
          MediaStage: true,
          MediaThumbnail: true,
          Inspector: true,
          ReviewControls: defineComponent({
            emits: ['patch'],
            setup(_, { emit }) {
              return () => h('button', { onClick: () => emit('patch', { pickState: 'keep' }) }, '标记保留');
            },
          }),
        },
      },
    });
    try {
      await flushPromises();
      failWrite = true;
      await wrapper
        .findAll('button')
        .find((button) => button.text() === '标记保留')!
        .trigger('click');
      await flushPromises();
      expect(wrapper.get('.viewer-photo-bar').text()).toContain('保存未确认');
      expect(wrapper.get('.viewer-photo-bar').text()).not.toContain('正在保存');
      expect(wrapper.get('[role="alert"]').text()).toContain('disk failure');
      const retry = wrapper.findAll('button').find((button) => button.text() === '重试')!;
      expect(retry.exists()).toBe(true);
      failWrite = false;
      await retry.trigger('click');
      await flushPromises();
      expect(wrapper.get('.viewer-photo-bar').text()).toContain('b.jpg');
      expect(wrapper.get('.viewer-photo-bar').text()).toContain('已保存到本机');
      expect(wrapper.find('[role="alert"]').exists()).toBe(false);
      expect(writes).toBe(1);
    } finally {
      wrapper.unmount();
    }
  });
});
describe('synchronized image geometry', () => {
  it('aligns the same relative image location across different resolutions at 1:1', () => {
    const a = stageGeometry({ width: 800, height: 600 }, { width: 6000, height: 4000 }, 1, true, {
      x: 0.1,
      y: -0.2,
    });
    const b = stageGeometry(
      { width: 800, height: 600 },
      { width: 3000, height: 2000 },
      1,
      true,
      a.normalized,
    );
    expect(a.x / a.width).toBeCloseTo(b.x / b.width);
    expect(a.y / a.height).toBeCloseTo(b.y / b.height);
    expect(a.x).toBe(600);
    expect(b.x).toBe(300);
  });
  it('clamps panning to the photo edges and resets displacement at fit', () => {
    const fit = stageGeometry({ width: 800, height: 600 }, { width: 6000, height: 4000 }, 1, false, {
      x: 3,
      y: 3,
    });
    expect(fit.x).toBe(0);
    expect(fit.y).toBe(0);
    const actual = stageGeometry({ width: 800, height: 600 }, { width: 6000, height: 4000 }, 1, true, {
      x: 3,
      y: -3,
    });
    expect(actual.x).toBe(2600);
    expect(actual.y).toBe(-1700);
  });
});
