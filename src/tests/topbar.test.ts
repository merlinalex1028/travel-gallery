import { afterEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { mount } from '@vue/test-utils';
import TopBar from '../components/TopBar.vue';
import { useLibrary } from '../stores/library';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, isDemo: true, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }));
afterEach(() => vi.useRealTimers());
it('does not treat a programmatic filter change as a typed search or change its route', async () => {
  setActivePinia(createPinia());
  const lib = useLibrary();
  lib.filter.keyword = 'old search';
  invoke.mockResolvedValue({ items: [], total: 0, nextCursor: null, queryFingerprint: 'search' });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/library', '/map'].map((path) => ({ path, component: { render: () => null } })),
  });
  await router.push('/map');
  vi.useFakeTimers();
  const wrapper = mount(TopBar, { global: { plugins: [router] } });
  lib.setFilter({ gpsCell: [43.47, 11.88] });
  await nextTick();
  await vi.advanceTimersByTimeAsync(250);
  expect(router.currentRoute.value.path).toBe('/map');
  expect(invoke).not.toHaveBeenCalled();
  expect(wrapper.get('input').element.value).toBe('');
  await wrapper.get('input').setValue('street');
  await vi.advanceTimersByTimeAsync(250);
  expect(router.currentRoute.value.path).toBe('/library');
  expect(lib.filter).toEqual({ gpsCell: [43.47, 11.88], keyword: 'street' });
  expect(invoke).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});
