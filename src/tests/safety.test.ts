import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { defineComponent, nextTick, shallowRef } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import type { AssetSummary, Page, SelectionSnapshot } from '../services/contracts';
import { photoKey } from '../composables/keyboard';
import { useLibrary } from '../stores/library';
import { usePreview, clearPreviewCache, prefetchPreview } from '../composables/preview';
import MediaGrid from '../components/MediaGrid.vue';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({
  api: invoke,
  errorMessage: (e: { message?: string }) => e.message ?? 'failed',
  resourceUrl: (url: string) => url,
}));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }));
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function asset(id: string): AssetSummary {
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
    width: 1,
    height: 1,
    durationMs: null,
    cameraModel: null,
    thumbnailState: 'queued',
    review: { pickState: 'unreviewed', rating: 0, isFavorite: false, note: '', revision: 0, updatedAt: 0 },
  };
}
function page(ids: string[], fp: string): Page<AssetSummary> {
  return { items: ids.map(asset), total: ids.length, nextCursor: null, queryFingerprint: fp };
}
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
  clearPreviewCache();
  document.body.innerHTML = '';
});

describe('query and immutable selection', () => {
  it('invalidates the previous detail and its pending response when changing scope', async () => {
    const response = deferred<unknown>();
    invoke.mockReturnValueOnce(response.promise);
    const lib = useLibrary();
    lib.focusId = 'a';
    const inspecting = lib.inspect('a');
    lib.setFilter({ gpsCell: [43.47, 11.88] });
    response.resolve({ ...asset('a'), tagIds: [] });
    await inspecting;
    expect(lib.detail).toBeNull();
    expect(lib.focusId).toBe('');
  });
  it('keeps a 50,000 item grid virtualized with a bounded visible DOM', async () => {
    vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockReturnValue(1200);
    vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(700);
    const wrapper = mount(MediaGrid, {
      attachTo: document.body,
      props: {
        items: Array.from({ length: 50000 }, (_, i) => asset(`photo-${i}`)),
        density: 220,
        hasMore: false,
        loadingMore: false,
        isSelected: () => false,
        groupBy: 'none',
      },
      global: { stubs: { MediaCard: { template: '<article class="tested-card" />' } } },
    });
    await nextTick();
    await flushPromises();
    const rendered = wrapper.findAll('.tested-card').length;
    expect(rendered).toBeGreaterThan(5);
    expect(rendered).toBeLessThan(100);
    wrapper.unmount();
    vi.restoreAllMocks();
  });
  it('does not let an older slow query replace a new result', async () => {
    const old = deferred<Page<AssetSummary>>(),
      recent = deferred<Page<AssetSummary>>();
    invoke.mockReturnValueOnce(old.promise).mockReturnValueOnce(recent.promise);
    const lib = useLibrary();
    const first = lib.search();
    lib.filter.keyword = 'new';
    const second = lib.search();
    recent.resolve(page(['new'], 'new'));
    await second;
    old.resolve(page(['old'], 'old'));
    await first;
    expect(lib.items.map((a) => a.id)).toEqual(['new']);
    expect(lib.fingerprint).toBe('new');
    expect(lib.loading).toBe(false);
  });
  it('freezes all matching without sending all asset IDs through the UI', async () => {
    const lib = useLibrary();
    const snapshot: SelectionSnapshot = {
      id: 'frozen',
      count: 50000,
      createdAt: 1,
      expiresAt: Date.now() + 10000,
      queryFingerprint: 'old-query',
    };
    invoke.mockResolvedValueOnce(snapshot);
    await lib.selectAllMatching();
    lib.setFilter({ pickStates: ['reject'] });
    expect(lib.count).toBe(50000);
    expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen', excludeAssetIds: [] });
    expect(invoke.mock.calls[0]?.[0]).toBe('create_selection_snapshot');
  });
  it('selects only loaded records for Cmd+A and supports a stable shift range', async () => {
    const lib = useLibrary();
    lib.items = ['a', 'b', 'c', 'd'].map(asset);
    lib.total = 50000;
    lib.selectLoaded();
    expect(lib.count).toBe(4);
    lib.clearSelection();
    invoke.mockResolvedValue({ ...asset('b'), tagIds: [] });
    lib.select(asset('b'));
    lib.select(asset('d'), new MouseEvent('click', { shiftKey: true }));
    expect([...lib.selected]).toEqual(['b', 'c', 'd']);
    await flushPromises();
  });
  it('rejects an expired snapshot without changing visible ratings', async () => {
    const lib = useLibrary();
    lib.items = [asset('a')];
    invoke.mockRejectedValueOnce({ code: 'SNAPSHOT_EXPIRED', message: 'expired' });
    await expect(lib.apply({ rating: 5 }, { kind: 'snapshot', snapshotId: 'expired' })).rejects.toMatchObject(
      { code: 'SNAPSHOT_EXPIRED' },
    );
    expect(lib.items[0]?.review.rating).toBe(0);
    expect(lib.history).toEqual([]);
    expect(lib.pending).toBe(0);
  });
});

describe('serialized writes and confirmed UI state', () => {
  it('serializes tag edits with reviews and undo while freezing the requested tag list', async () => {
    const lib = useLibrary();
    lib.items = [asset('a')];
    const tags = deferred<unknown>();
    invoke.mockImplementation((name: string) => {
      if (name === 'apply_tags_batch') return tags.promise;
      if (name === 'apply_review_batch')
        return Promise.resolve({
          changeBatchId: 'review',
          affectedCount: 1,
          unchangedCount: 0,
          conflicts: [],
        });
      if (name === 'undo_review_batch') return Promise.resolve({ restoredCount: 1, skippedCount: 0 });
      if (name === 'query_assets')
        return Promise.resolve({ items: [asset('a')], total: 1, nextCursor: null, queryFingerprint: 'q' });
      throw new Error(name);
    });
    const names = ['t'];
    const tagging = lib.applyTags(names, [], { kind: 'explicit', assetIds: ['a'] });
    names.push('late');
    const review = lib.apply({ rating: 4 }, { kind: 'explicit', assetIds: ['a'] });
    const one = lib.undo(),
      two = lib.undo();
    await flushPromises();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(['apply_tags_batch']);
    expect(invoke.mock.calls[0]?.[1].addTagIds).toEqual(['t']);
    tags.resolve({ changeBatchId: 'tags', affectedCount: 1, unchangedCount: 0, conflicts: [] });
    await Promise.all([tagging, review, one, two]);
    expect(invoke.mock.calls.filter((c) => c[0] === 'undo_review_batch').map((c) => c[1])).toEqual([
      { changeBatchId: 'review' },
      { changeBatchId: 'tags' },
    ]);
    expect(lib.history).toEqual([]);
    expect(lib.pending).toBe(0);
  });
  it('queues repeated undo after an in-flight write and chooses each latest batch at execution', async () => {
    const lib = useLibrary();
    lib.items = [asset('a')];
    lib.history = ['older'];
    const write = deferred<unknown>();
    invoke.mockImplementation((name: string) => {
      if (name === 'apply_review_batch') return write.promise;
      if (name === 'undo_review_batch') return Promise.resolve({ restoredCount: 1, skippedCount: 0 });
      if (name === 'query_assets')
        return Promise.resolve({ items: [asset('a')], total: 1, nextCursor: null, queryFingerprint: 'q' });
      throw new Error(name);
    });
    const saving = lib.apply({ rating: 4 }, { kind: 'explicit', assetIds: ['a'] });
    const one = lib.undo(),
      two = lib.undo();
    await flushPromises();
    expect(invoke.mock.calls.map((c) => c[0])).toEqual(['apply_review_batch']);
    write.resolve({ changeBatchId: 'newer', affectedCount: 1, unchangedCount: 0, conflicts: [] });
    await Promise.all([saving, one, two]);
    expect(invoke.mock.calls.filter((c) => c[0] === 'undo_review_batch').map((c) => c[1])).toEqual([
      { changeBatchId: 'newer' },
      { changeBatchId: 'older' },
    ]);
    expect(lib.history).toEqual([]);
    expect(lib.pending).toBe(0);
  });
  it('serializes rapid mutations and waits for the backend receipt', async () => {
    const lib = useLibrary();
    lib.items = [asset('a')];
    const first = deferred<unknown>(),
      second = deferred<unknown>();
    invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const one = lib.apply({ pickState: 'keep' }, { kind: 'explicit', assetIds: ['a'] });
    const two = lib.apply({ rating: 4 }, { kind: 'explicit', assetIds: ['a'] });
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(lib.items[0]?.review.pickState).toBe('unreviewed');
    first.resolve({ changeBatchId: 'one', affectedCount: 1, unchangedCount: 0, conflicts: [] });
    await one;
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(2);
    second.resolve({ changeBatchId: 'two', affectedCount: 1, unchangedCount: 0, conflicts: [] });
    await two;
    expect(lib.items[0]?.review.pickState).toBe('keep');
    expect(lib.items[0]?.review.rating).toBe(4);
    expect(lib.history).toEqual(['one', 'two']);
    expect(lib.pending).toBe(0);
    expect(invoke.mock.calls[0]?.[1].clientMutationId).not.toBe(invoke.mock.calls[1]?.[1].clientMutationId);
  });
  it('does not poison the queue after a failed write', async () => {
    const lib = useLibrary();
    lib.items = [asset('a')];
    invoke
      .mockRejectedValueOnce({ message: 'disk' })
      .mockResolvedValueOnce({ changeBatchId: 'saved', affectedCount: 1, unchangedCount: 0, conflicts: [] });
    await expect(lib.apply({ rating: 5 }, { kind: 'explicit', assetIds: ['a'] })).rejects.toMatchObject({
      message: 'disk',
    });
    await lib.apply({ rating: 2 }, { kind: 'explicit', assetIds: ['a'] });
    expect(lib.items[0]?.review.rating).toBe(2);
  });
});

describe('preview response races', () => {
  it('keeps B visible when A finishes later', async () => {
    const a = deferred<unknown>(),
      b = deferred<unknown>();
    invoke.mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    const id = shallowRef('a');
    const Host = defineComponent({
      setup() {
        return usePreview(id, 'screen');
      },
      template: '<span>{{ url }}</span>',
    });
    const wrapper = mount(Host);
    await flushPromises();
    id.value = 'b';
    await nextTick();
    b.resolve({ resourceUrl: 'b.jpg', ready: true });
    await flushPromises();
    expect(wrapper.text()).toBe('b.jpg');
    a.resolve({ resourceUrl: 'a.jpg', ready: true });
    await flushPromises();
    expect(wrapper.text()).toBe('b.jpg');
    wrapper.unmount();
  });
  it('caps active requests and cancels obsolete queued navigation', async () => {
    const pending: ReturnType<typeof deferred<unknown>>[] = [];
    invoke.mockImplementation(() => {
      const request = deferred<unknown>();
      pending.push(request);
      return request.promise;
    });
    const ids = Array.from({ length: 4 }, (_, i) => shallowRef(`bounded-${i}`));
    const wrappers = ids.map((id) =>
      mount(
        defineComponent({
          setup: () => usePreview(id, 'screen'),
          template: '<span>{{ url }}</span>',
        }),
      ),
    );
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(3);
    for (let i = 0; i < 30; i++) {
      ids[3]!.value = `rapid-${i}`;
      await nextTick();
    }
    expect(invoke).toHaveBeenCalledTimes(3);
    pending[0]!.resolve({ ready: false });
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(4);
    expect(invoke.mock.calls[3]?.[1].assetId).toBe('rapid-29');
    wrappers.forEach((wrapper) => wrapper.unmount());
    pending.forEach((request) => request.resolve({ ready: false }));
    await flushPromises();
  });
  it('runs visible previews before queued neighbor prefetches', async () => {
    const pending: ReturnType<typeof deferred<unknown>>[] = [];
    invoke.mockImplementation(() => {
      const request = deferred<unknown>();
      pending.push(request);
      return request.promise;
    });
    for (let i = 0; i < 4; i++) prefetchPreview(`background-${i}`);
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(2);
    prefetchPreview('queued-neighbor');
    const wrapper = mount(
      defineComponent({
        setup: () => usePreview('visible-first', 'screen'),
        template: '<span>{{ url }}</span>',
      }),
    );
    await flushPromises();
    expect(invoke.mock.calls[2]?.[1].assetId).toBe('visible-first');
    wrapper.unmount();
    clearPreviewCache();
    pending.forEach((request) => request.resolve({ ready: false }));
    await flushPromises();
  });
  it('displays a cached small preview without waiting for three slow large previews', async () => {
    const pendingLarge: ReturnType<typeof deferred<unknown>>[] = [];
    invoke.mockImplementation((_command, request) => {
      if (request.variant === 'small')
        return Promise.resolve({ ready: true, resourceUrl: 'current-small.jpg' });
      const item = deferred<unknown>();
      pendingLarge.push(item);
      return item.promise;
    });
    const large = Array.from({ length: 3 }, (_, i) =>
      mount(
        defineComponent({
          setup: () => usePreview(`slow-${i}`, 'screen'),
          template: '<span>{{ url }}</span>',
        }),
      ),
    );
    await flushPromises();
    const small = mount(
      defineComponent({
        setup: () => usePreview('current', 'small'),
        template: '<img v-if="url" :src="url" alt="current" />',
      }),
    );
    await flushPromises();
    expect(small.get('img[alt="current"]').attributes('src')).toBe('current-small.jpg');
    expect(pendingLarge).toHaveLength(3);
    large.forEach((wrapper) => wrapper.unmount());
    small.unmount();
    pendingLarge.forEach((item) => item.resolve({ ready: false }));
    await flushPromises();
  });
  it('reloads mounted previews on cache invalidation and rejects stale cache responses', async () => {
    const old = deferred<unknown>(),
      fresh = deferred<unknown>();
    invoke.mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise);
    const Host = defineComponent({
      setup: () => usePreview('same-file', 'screen'),
      template: '<span>{{ url }}</span>',
    });
    const wrapper = mount(Host);
    await flushPromises();
    clearPreviewCache();
    await flushPromises();
    fresh.resolve({ ready: true, resourceUrl: 'fresh.jpg' });
    await flushPromises();
    old.resolve({ ready: true, resourceUrl: 'obsolete.jpg' });
    await flushPromises();
    expect(wrapper.text()).toBe('fresh.jpg');
    const second = mount(Host);
    await flushPromises();
    expect(second.text()).toBe('fresh.jpg');
    expect(invoke).toHaveBeenCalledTimes(2);
    wrapper.unmount();
    second.unmount();
  });
  it('retries a failed cached resource once without an infinite reload loop', async () => {
    invoke.mockResolvedValue({ ready: true, resourceUrl: 'image.jpg' });
    const wrapper = mount(
      defineComponent({
        setup: () => usePreview('retry-file', 'screen'),
        template: '<button @click="retry">{{ url || reason }}</button>',
      }),
    );
    await flushPromises();
    await wrapper.trigger('click');
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(2);
    await wrapper.trigger('click');
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain('预览加载失败');
    wrapper.unmount();
  });
});

describe('keyboard scope', () => {
  function event(key: string, element?: HTMLElement, options: KeyboardEventInit = {}) {
    const e = new KeyboardEvent('keydown', { key, ...options });
    if (element) Object.defineProperty(e, 'target', { value: element });
    return e;
  }
  it('ignores text inputs, editable content, menus and dialogs', () => {
    for (const selector of ['input', 'textarea', 'select'])
      expect(photoKey(event('p', document.createElement(selector)))).toBeNull();
    const editable = document.createElement('div');
    editable.contentEditable = 'true';
    editable.setAttribute('contenteditable', 'true');
    expect(photoKey(event('x', editable))).toBeNull();
    const dialog = document.createElement('div');
    dialog.setAttribute('role', 'dialog');
    expect(photoKey(event('5', dialog))).toBeNull();
  });
  it('ignores auto-repeat marking and respects system modifiers', () => {
    expect(photoKey(event('p', undefined, { repeat: true }))).toBeNull();
    expect(photoKey(event('ArrowRight', undefined, { repeat: true }))).toBe('arrowright');
    expect(photoKey(event('p', undefined, { metaKey: true }))).toBeNull();
    expect(photoKey(event('z', undefined, { metaKey: true }))).toBe('undo');
    expect(photoKey(event('U'))).toBe('u');
  });
  it('leaves native media, shadow controls, sliders and consumed events in their own scope', () => {
    for (const tag of ['video', 'audio']) {
      const media = document.createElement(tag);
      for (const key of [' ', 'ArrowRight', 'ArrowLeft', 'f', '5'])
        expect(photoKey(event(key, media))).toBeNull();
    }
    const slider = document.createElement('div');
    slider.setAttribute('role', 'slider');
    const icon = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    slider.append(icon);
    const e = event('ArrowRight');
    Object.defineProperty(e, 'target', { value: icon });
    expect(photoKey(e)).toBeNull();
    const shadowEvent = event('f');
    vi.spyOn(shadowEvent, 'composedPath').mockReturnValue([document.createElement('video')]);
    expect(photoKey(shadowEvent)).toBeNull();
    const consumed = event('f', undefined, { cancelable: true });
    consumed.preventDefault();
    expect(photoKey(consumed)).toBeNull();
    expect(photoKey(event('p', undefined, { isComposing: true }))).toBeNull();
    expect(photoKey(event(' ', document.createElement('button')))).toBeNull();
    expect(photoKey(event('f', document.createElement('div')))).toBe('f');
  });
  it('ignores global photo keys when an overlay is open', () => {
    const overlay = document.createElement('div');
    overlay.dataset.modal = 'true';
    document.body.append(overlay);
    expect(photoKey(event('x'))).toBeNull();
  });
  it('restores shortcuts when Element Plus retains a closed hidden overlay', () => {
    const overlay = document.createElement('div');
    overlay.style.display = 'none';
    overlay.innerHTML = '<div class="el-overlay-dialog"><div role="dialog">Closed import dialog</div></div>';
    document.body.append(overlay);
    expect(photoKey(event('f'))).toBe('f');
    overlay.style.display = '';
    expect(photoKey(event('f'))).toBeNull();
    overlay.setAttribute('aria-hidden', 'true');
    expect(photoKey(event('f'))).toBe('f');
    overlay.remove();
  });
});
