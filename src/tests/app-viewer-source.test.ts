import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { defineComponent } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { flushPromises, mount } from '@vue/test-utils';
import App from '../App.vue';
import { useViewer } from '../composables/viewer';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import type { AssetDetail } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({
  api: invoke,
  native: false,
  resourceUrl: (value: string) => value,
  errorMessage: (error: Error) => error.message,
}));
vi.mock('element-plus', async (original) => ({
  ...(await original<typeof import('element-plus')>()),
  ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
}));
let records: AssetDetail[];
let failWrite: boolean;
const mounted: { unmount(): void }[] = [];
beforeEach(() => {
  vi.useFakeTimers();
  setActivePinia(createPinia());
  records = ['a', 'b'].map((id) => ({
    ...(structuredClone(golden.assetDetail) as AssetDetail),
    id,
    sourceId: 'source',
    filename: `${id}.jpg`,
    fileState: 'available',
    review: { pickState: 'unreviewed', rating: 0, isFavorite: false, note: '', revision: 0, updatedAt: 0 },
  }));
  failWrite = false;
  invoke.mockReset();
  invoke.mockImplementation(async (command: string, request: any) => {
    if (command === 'get_asset') return structuredClone(records.find((r) => r.id === request.assetId));
    if (command === 'get_asset_neighbors')
      return {
        previousId: request.assetId === 'b' ? 'a' : null,
        nextId: request.assetId === 'a' ? 'b' : null,
        items: structuredClone(records),
        total: 2,
        position: request.assetId === 'a' ? 1 : 2,
        anchorMatches: true,
      };
    if (command === 'save_session') return { updated: true };
    if (command === 'get_cull_progress')
      return {
        scope: { total: 2, reviewed: 0 },
        day: { label: 'day', total: 2, reviewed: 0 },
        folder: { label: 'folder', total: 2, reviewed: 0 },
      };
    if (command === 'apply_review_batch') {
      if (failWrite) throw new Error('write failed');
      const record = records.find((r) => r.id === request.target.assetIds[0])!;
      Object.assign(record.review, request.patch);
      record.review.revision++;
      return { changeBatchId: 'batch', affectedCount: 1, unchangedCount: 0, conflicts: [] };
    }
    throw new Error(`Unexpected command ${command}`);
  });
});
afterEach(() => {
  mounted
    .splice(0)
    .reverse()
    .forEach((wrapper) => wrapper.unmount());
  vi.useRealTimers();
  vi.restoreAllMocks();
});
async function setup(cull = false) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/viewer/:assetId', '/cull', '/library'].map((path) => ({
      path,
      component: { render: () => null },
    })),
  });
  await router.push(cull ? '/cull' : '/viewer/a');
  await router.isReady();
  const app = useApp(),
    lib = useLibrary();
  app.ready = true;
  app.settings.prefetch = 0;
  app.settings.autoAdvance = true;
  app.sources = [
    {
      id: 'source',
      displayName: 'Source',
      absolutePath: '/source',
      volumeId: null,
      isEnabled: true,
      isOnline: true,
      lastScanAt: 0,
      totalAssets: 2,
      offlineAssets: 0,
    },
  ];
  lib.total = 2;
  lib.session = {
    filter: {},
    sort: { field: 'filename', direction: 'asc' },
    anchorAssetId: 'a',
    queryFingerprint: 'query',
    position: 1,
    total: 2,
    returnPath: '/library',
  };
  vi.spyOn(app, 'init').mockResolvedValue(undefined);
  let effect = () => {
    app.catalogEpoch++;
    app.sources = [{ ...app.sources[0]!, lastScanAt: (app.sources[0]!.lastScanAt ?? 0) + 1 }];
  };
  vi.spyOn(app, 'refresh').mockImplementation(async () => effect());
  let viewer!: ReturnType<typeof useViewer>;
  mounted.push(
    mount(
      defineComponent({
        setup() {
          viewer = useViewer();
          return () => null;
        },
      }),
      { global: { plugins: [router] } },
    ),
  );
  await flushPromises();
  const shell = mount(App, {
    global: {
      plugins: [router],
      stubs: {
        ElConfigProvider: { props: ['locale'], template: '<slot />' },
        MainNav: true,
        TopBar: true,
        GlobalDialogs: true,
        RouterView: true,
        Icon: true,
      },
    },
  });
  mounted.push(shell);
  await flushPromises();
  const poll = async (next?: () => void) => {
    if (next) effect = next;
    await vi.advanceTimersByTimeAsync(1500);
    await flushPromises();
  };
  return { viewer, app, lib, poll };
}
it('refreshes missing and restored state in the open viewer while retaining zoom and position', async () => {
  const { viewer, lib, poll } = await setup();
  viewer.zoom.value = 2;
  viewer.actualPixels.value = true;
  records[0]!.fileState = 'missing';
  await poll();
  expect(viewer.asset.value?.fileState).toBe('missing');
  expect(lib.detail?.fileState).toBe('missing');
  expect(viewer.asset.value?.id).toBe('a');
  expect(viewer.zoom.value).toBe(2);
  expect(viewer.actualPixels.value).toBe(true);
  expect(viewer.position.value).toBe(1);
  records[0]!.fileState = 'available';
  await poll();
  expect(viewer.asset.value?.fileState).toBe('available');
  expect(invoke.mock.calls.some(([command]) => command === 'apply_review_batch')).toBe(false);
});
it('refreshes an offline source and reconnect even if the catalog epoch remains unchanged', async () => {
  const { viewer, app, poll } = await setup();
  records[0]!.fileState = 'offline';
  await poll(() => {
    app.sources = [{ ...app.sources[0]!, isOnline: false }];
  });
  expect(viewer.asset.value?.fileState).toBe('offline');
  records[0]!.fileState = 'available';
  await poll(() => {
    app.sources = [{ ...app.sources[0]!, isOnline: true }];
  });
  expect(viewer.asset.value?.fileState).toBe('available');
});
it('preserves the failed review intent, idempotency key and frozen progress during a source refresh', async () => {
  const { viewer, poll } = await setup(true);
  failWrite = true;
  await viewer.mark({ pickState: 'keep' });
  const original = invoke.mock.calls.find(([command]) => command === 'apply_review_batch')![1];
  records[0]!.fileState = 'missing';
  await poll();
  expect(viewer.asset.value?.fileState).toBe('missing');
  expect(viewer.canRetry.value).toBe(true);
  expect(viewer.saved.value).toBe(false);
  expect(viewer.error.value).toBe('write failed');
  expect(viewer.position.value).toBe(1);
  expect(viewer.initialTotal.value).toBe(2);
  expect(viewer.processed.value).toBe(0);
  failWrite = false;
  await viewer.retryReview();
  const attempts = invoke.mock.calls.filter(([command]) => command === 'apply_review_batch');
  expect(attempts).toHaveLength(2);
  expect(attempts[1]![1]).toEqual(original);
  expect(viewer.asset.value?.id).toBe('b');
  expect(viewer.processed.value).toBe(1);
});
it('discards a late detail poll after navigating to another photo', async () => {
  const { viewer, lib, poll } = await setup();
  const original = invoke.getMockImplementation()!;
  let finish!: (value: AssetDetail) => void;
  const pending = new Promise<AssetDetail>((resolve) => {
    finish = resolve;
  });
  invoke.mockImplementation((command: string, request: any) =>
    command === 'get_asset' && request.assetId === 'a' ? pending : original(command, request),
  );
  await poll();
  expect(invoke.mock.calls.filter(([command]) => command === 'get_asset')).toHaveLength(2);
  await viewer.load('b');
  finish({ ...records[0]!, fileState: 'missing' });
  await flushPromises();
  expect(viewer.asset.value?.id).toBe('b');
  expect(lib.detail?.id).toBe('b');
  expect(lib.focusId).toBe('b');
});
it('does not replace a newer review with an older background read', async () => {
  const { viewer, lib, poll } = await setup();
  let finish!: (value: AssetDetail) => void;
  const pending = new Promise<AssetDetail>((resolve) => {
    finish = resolve;
  });
  invoke.mockImplementation((command: string) => {
    if (command === 'get_asset') return pending;
    throw new Error(`Unexpected command ${command}`);
  });
  await poll();
  expect(invoke.mock.calls.filter(([command]) => command === 'get_asset')).toHaveLength(2);
  lib.detail = { ...records[0]!, review: { ...records[0]!.review, revision: 5, note: 'newer edit' } };
  await flushPromises();
  finish({ ...records[0]!, fileState: 'missing' });
  await flushPromises();
  expect(viewer.asset.value?.review.note).toBe('newer edit');
  expect(lib.detail?.review.revision).toBe(5);
});
it('retains the current cached photo on a read failure and retries at the next poll', async () => {
  const { viewer, poll } = await setup();
  const original = invoke.getMockImplementation()!;
  invoke.mockRejectedValueOnce(new Error('temporary detail failure'));
  viewer.zoom.value = 3;
  await poll();
  expect(viewer.asset.value?.id).toBe('a');
  expect(viewer.zoom.value).toBe(3);
  expect(viewer.saved.value).toBe(true);
  expect(viewer.loading.value).toBe(false);
  invoke.mockImplementation(original);
  records[0]!.fileState = 'missing';
  await poll(() => {});
  expect(viewer.asset.value?.fileState).toBe('missing');
});
it('bounds background read retries until another source change arrives', async () => {
  const { viewer, poll } = await setup();
  invoke.mockRejectedValue(new Error('persistent detail failure'));
  await poll();
  await poll(() => {});
  await poll(() => {});
  expect(invoke.mock.calls.filter(([command]) => command === 'get_asset')).toHaveLength(3);
  expect(viewer.asset.value?.id).toBe('a');
  expect(viewer.saved.value).toBe(true);
});
