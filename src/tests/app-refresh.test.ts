import { afterEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { mount } from '@vue/test-utils';
import App from '../App.vue';
import { useLibrary } from '../stores/library';
import golden from '../../tests/fixtures/contracts.json';
import type { AssetDetail } from '../services/contracts';
const { invoke, clear } = vi.hoisted(() => ({ invoke: vi.fn(), clear: vi.fn() }));
vi.mock('../services/api', () => ({ native: true, api: invoke, errorMessage: () => 'failed' }));
vi.mock('../composables/preview', () => ({ clearPreviewCache: clear }));
vi.mock('element-plus', () => ({
  ElConfigProvider: { props: ['locale'], template: '<slot />' },
  ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
}));
afterEach(() => vi.useRealTimers());
it('refreshes a scan completed between polls, then refreshes metadata changes without discarding previews', async () => {
  setActivePinia(createPinia());
  let epoch = 1,
    scanned = false;
  invoke.mockImplementation(async (name: string) => {
    if (name === 'get_settings') return {};
    if (name === 'list_sources')
      return [
        {
          id: 's',
          displayName: 'fixture',
          absolutePath: '/fixture',
          isEnabled: true,
          isOnline: true,
          lastScanAt: scanned ? 2 : 1,
          totalAssets: scanned ? 2 : 1,
          offlineAssets: 0,
        },
      ];
    if (name === 'list_jobs')
      return { items: scanned ? [{ id: 'fast-scan', type: 'SCAN', status: 'completed', updatedAt: 2 }] : [] };
    if (['list_tags', 'list_trips', 'list_saved_filters'].includes(name)) return [];
    if (name === 'get_app_status')
      return {
        catalogEpoch: epoch,
        watcher: { active: true, watchedRoots: 1, pollingRoots: 0, lastError: null },
      };
    if (name === 'query_assets')
      return { items: [], total: 2, nextCursor: null, queryFingerprint: `epoch-${epoch}` };
    if (name === 'get_asset') return { ...golden.assetDetail, width: 6001 };
    throw new Error(`Unexpected ${name}`);
  });
  clear.mockClear();
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/library', component: { render: () => null } }],
  });
  await router.push('/library');
  vi.useFakeTimers();
  const wrapper = mount(App, {
    global: {
      plugins: [router],
      stubs: { MainNav: true, TopBar: true, GlobalDialogs: true, RouterView: true, AppUpdates: true },
    },
  });
  await vi.advanceTimersByTimeAsync(0);
  await nextTick();
  useLibrary().detail = golden.assetDetail as AssetDetail;
  useLibrary().focusId = golden.assetDetail.id;
  scanned = true;
  epoch = 2;
  await vi.advanceTimersByTimeAsync(1500);
  await nextTick();
  expect(invoke.mock.calls.filter(([name]) => name === 'query_assets')).toHaveLength(1);
  expect(useLibrary().total).toBe(2);
  expect(clear).toHaveBeenCalledTimes(1);
  expect(useLibrary().detail?.width).toBe(6001);
  epoch = 3;
  await vi.advanceTimersByTimeAsync(1500);
  await nextTick();
  expect(invoke.mock.calls.filter(([name]) => name === 'query_assets')).toHaveLength(2);
  expect(clear).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});
