import { setSelect } from './helpers/select';
import { expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, shallowMount } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import BurstGroups from '../features/BurstGroups.vue';
import LibraryPage from '../pages/LibraryPage.vue';
import { useLibrary } from '../stores/library';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, isDemo: false, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({
  ElMessageBox: {},
  ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() },
}));
async function setup(path = '/duplicates') {
  setActivePinia(createPinia());
  invoke.mockReset();
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/duplicates', '/filter', '/trips/:id', '/library'].map((path) => ({
      path,
      component: { render: () => null },
    })),
  });
  await router.push(path);
  return { router, lib: useLibrary() };
}
it('keeps only the latest shooting query and opens all members through a frozen snapshot', async () => {
  const { router, lib } = await setup();
  let resolveOld!: (value: unknown) => void;
  const group = {
    id: 'group-all-20',
    sourceId: 's',
    folder: '',
    cameraModel: 'Camera',
    captureTzMin: null,
    startLocal: '2026-06-15 09:00:00',
    endLocal: '2026-06-15 09:00:19',
    memberCount: 20,
    members: [],
  };
  invoke.mockImplementation((name: string, r: { gapSeconds?: number }) => {
    if (name === 'query_burst_groups')
      return r.gapSeconds === 2
        ? new Promise((resolve) => {
            resolveOld = resolve;
          })
        : Promise.resolve({ items: [group], total: 1, nextCursor: null, examinedCount: 20 });
    if (name === 'create_burst_snapshot') return Promise.resolve({ id: 'frozen-20', count: 20 });
    throw new Error(name);
  });
  const wrapper = mount(BurstGroups, { global: { plugins: [router], stubs: { MediaThumbnail: true } } });
  await setSelect(wrapper, '拍摄组相邻间隔', 5);
  await flushPromises();
  resolveOld({ items: [], total: 0, nextCursor: null, examinedCount: 0 });
  await flushPromises();
  expect(wrapper.text()).toContain('20 张');
  lib.selected = new Set(['old']);
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '打开整组挑选')!
    .trigger('click');
  await flushPromises();
  expect(router.currentRoute.value.fullPath).toBe('/filter?showResults=1');
  expect(lib.filter).toEqual({ selectionSnapshotId: 'frozen-20' });
  expect(lib.selected.size).toBe(0);
  expect(invoke).toHaveBeenCalledWith('create_burst_snapshot', {
    filter: {},
    gapSeconds: 5,
    groupId: 'group-all-20',
  });
  wrapper.unmount();
});
it('preserves a trip filter when the map drills into the shared results page', async () => {
  const { router, lib } = await setup('/filter?showResults=1');
  lib.setFilter({ collectionId: 'trip', gpsCell: [43.47, 11.88] });
  invoke.mockResolvedValue({ items: [], total: 0, nextCursor: null, queryFingerprint: 'trip-cell' });
  const wrapper = shallowMount(LibraryPage, { global: { plugins: [router] } });
  await flushPromises();
  expect(lib.filter).toEqual({ collectionId: 'trip', gpsCell: [43.47, 11.88] });
  expect(invoke).toHaveBeenCalledWith(
    'query_assets',
    expect.objectContaining({ filter: { collectionId: 'trip', gpsCell: [43.47, 11.88] } }),
  );
  await router.push('/trips/second');
  await flushPromises();
  expect(lib.filter).toEqual({ collectionId: 'second' });
  await router.push('/library');
  await flushPromises();
  expect(lib.filter).toEqual({});
  wrapper.unmount();
});
