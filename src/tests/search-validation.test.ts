import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import TopBar from '../components/TopBar.vue';
import { useLibrary } from '../stores/library';

const { invoke, push } = vi.hoisted(() => ({ invoke: vi.fn(), push: vi.fn() }));
vi.mock('../services/api', () => ({ api: invoke, isDemo: false, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), error: vi.fn() } }));
vi.mock('vue-router', () => ({ useRoute: () => ({ path: '/library' }), useRouter: () => ({ push }) }));
beforeEach(() => {
  setActivePinia(createPinia());
  vi.useFakeTimers();
  invoke.mockReset().mockResolvedValue({ items: [], total: 0, nextCursor: null, queryFingerprint: 'fresh' });
  push.mockReset();
});
afterEach(() => vi.useRealTimers());
function topbar() {
  return mount(TopBar, { global: { stubs: { Icon: true } } });
}

it.each(['a'.repeat(2001), '📷'.repeat(501)])(
  'keeps an oversized search draft out of the active query and preserves its frozen range',
  async (text) => {
    const lib = useLibrary();
    lib.filter.keyword = 'JPEG';
    lib.snapshot = { id: 'frozen-two', count: 2, createdAt: 1, expiresAt: 100, queryFingerprint: 'original' };
    const target = { ...lib.target };
    const wrapper = topbar();
    const input = wrapper.get('[aria-label="搜索媒体"]');
    await input.setValue(text);
    await vi.advanceTimersByTimeAsync(300);
    expect(invoke).not.toHaveBeenCalled();
    expect(push).not.toHaveBeenCalled();
    expect(lib.filter.keyword).toBe('JPEG');
    expect(lib.target).toEqual(target);
    expect(input.attributes('aria-invalid')).toBe('true');
    expect(wrapper.get(`[id="${input.attributes('aria-describedby')}"]`).text()).toContain('搜索内容过长');
    await input.setValue('HEIC');
    await vi.advanceTimersByTimeAsync(300);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(lib.filter.keyword).toBe('HEIC');
    expect(lib.target).toEqual(target);
    expect(input.attributes('aria-invalid')).toBeUndefined();
    wrapper.unmount();
  },
);

it.each(['a'.repeat(2000), '📷'.repeat(500)])(
  'accepts the exact backend UTF8 byte boundary',
  async (text) => {
    const wrapper = topbar();
    await wrapper.get('[aria-label="搜索媒体"]').setValue(text);
    await vi.advanceTimersByTimeAsync(300);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(useLibrary().filter.keyword).toBe(text);
    expect(wrapper.find('[aria-invalid="true"]').exists()).toBe(false);
    wrapper.unmount();
  },
);

it('cancels an earlier scheduled search when the latest input exceeds the byte limit', async () => {
  const wrapper = topbar();
  const input = wrapper.get('[aria-label="搜索媒体"]');
  await input.setValue('pending');
  await input.setValue('📷'.repeat(501));
  await vi.advanceTimersByTimeAsync(500);
  expect(invoke).not.toHaveBeenCalled();
  expect(useLibrary().filter.keyword).toBeUndefined();
  wrapper.unmount();
});
