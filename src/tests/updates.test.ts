import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { nextTick } from 'vue';
import { useUpdates } from '../stores/updates';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { settingsWithDefaults } from '../services/settings';
import AppUpdates from '../components/AppUpdates.vue';
import AppUpdatePanel from '../components/AppUpdatePanel.vue';

const mocks = vi.hoisted(() => ({
  support: vi.fn(),
  check: vi.fn(),
  download: vi.fn(),
  install: vi.fn(),
  confirm: vi.fn(),
  api: vi.fn(),
}));
vi.mock('../services/updater', () => ({ updater: mocks }));
vi.mock('../services/api', () => ({
  api: mocks.api,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));
vi.mock('vue-router', () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock('element-plus', () => ({ ElMessage: { error: vi.fn() }, ElMessageBox: { confirm: mocks.confirm } }));
const wrappers: ReturnType<typeof mount>[] = [];
beforeEach(() => {
  vi.resetAllMocks();
  setActivePinia(createPinia());
  mocks.support.mockResolvedValue({ supported: true, version: '1.0.0' });
  mocks.check.mockResolvedValue({ version: '1.1.0', notes: '发布说明' });
  mocks.download.mockResolvedValue(undefined);
  mocks.install.mockResolvedValue(undefined);
  mocks.confirm.mockResolvedValue('confirm');
  mocks.api.mockResolvedValue({ updated: true });
});
afterEach(() => {
  wrappers.splice(0).forEach((w) => w.unmount());
  vi.useRealTimers();
});
function panel() {
  const wrapper = mount(AppUpdatePanel, {
    global: {
      stubs: {
        ElSwitch: {
          props: ['modelValue', 'disabled'],
          emits: ['update:modelValue'],
          template:
            '<button :disabled="disabled" aria-label="自动检查更新" @click="$emit(\'update:modelValue\', !modelValue)">{{ modelValue }}</button>',
        },
      },
    },
  });
  wrappers.push(wrapper);
  return wrapper;
}
async function downloaded() {
  const store = useUpdates();
  await store.check();
  await store.download();
  return store;
}

it('keeps legacy preferences offline by default', () => {
  expect(settingsWithDefaults({ theme: 'dark' }).autoCheckUpdates).toBe(false);
});
it('never checks or downloads from an unsupported browser/development runtime', async () => {
  mocks.support.mockResolvedValue({ supported: false, version: '1.0.0' });
  const store = useUpdates();
  await store.check();
  await store.download();
  await store.install();
  expect(mocks.check).not.toHaveBeenCalled();
  expect(mocks.download).not.toHaveBeenCalled();
  expect(mocks.install).not.toHaveBeenCalled();
});
it('distinguishes a successful up-to-date result from a failed check', async () => {
  const store = useUpdates();
  mocks.check.mockResolvedValue(null);
  await store.check();
  expect(store.phase).toBe('current');
  expect(store.lastChecked).not.toBeNull();
  const checked = store.lastChecked;
  mocks.check.mockRejectedValue(new Error('网络不可用'));
  await store.check();
  expect(store.phase).toBe('idle');
  expect(store.error).toBe('网络不可用');
  expect(store.lastChecked).toBe(checked);
});
it('serializes checks and never downloads automatically after detection', async () => {
  let resolve!: (value: null) => void;
  mocks.check.mockImplementation(
    () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  const store = useUpdates(),
    first = store.check();
  await flushPromises();
  await store.check();
  expect(mocks.check).toHaveBeenCalledTimes(1);
  resolve(null);
  await first;
  expect(mocks.download).not.toHaveBeenCalled();
});
it('reports progress, retries failed signatures/downloads, and installs only verified ready packages', async () => {
  const store = useUpdates();
  await store.check();
  await store.install();
  expect(mocks.install).not.toHaveBeenCalled();
  mocks.download.mockImplementationOnce(async (progress) => {
    progress({ downloaded: 50, total: 200 });
    throw new Error('签名失败');
  });
  await store.download();
  expect(store.phase).toBe('available');
  expect(store.progress).toBe(25);
  expect(store.error).toBe('签名失败');
  await store.install();
  expect(mocks.install).not.toHaveBeenCalled();
  mocks.download.mockImplementationOnce(async (progress) => progress({ downloaded: 200, total: 200 }));
  await store.download();
  expect(store.phase).toBe('ready');
  expect(store.progress).toBe(100);
  expect(store.error).toBe('');
  await store.check();
  expect(mocks.check).toHaveBeenCalledTimes(1);
  mocks.install.mockRejectedValueOnce(new Error('任务未完成'));
  await store.install();
  expect(store.phase).toBe('ready');
  expect(store.error).toBe('任务未完成');
  await store.install();
  expect(store.phase).toBe('installing');
  expect(mocks.install).toHaveBeenCalledTimes(2);
});
it('supports downloads whose length is unknown', async () => {
  mocks.download.mockImplementationOnce(async (progress) => progress({ downloaded: 100, total: null }));
  const store = await downloaded();
  expect(store.progress).toBeNull();
  expect(store.phase).toBe('ready');
});
it('starts automatic checks only after opt-in and stops checking after opt-out or unmount', async () => {
  vi.useFakeTimers();
  const app = useApp();
  app.ready = true;
  const wrapper = mount(AppUpdates, { global: { stubs: { ElDialog: true } } });
  wrappers.push(wrapper);
  await flushPromises();
  expect(mocks.check).not.toHaveBeenCalled();
  app.settings.autoCheckUpdates = true;
  await flushPromises();
  expect(mocks.check).toHaveBeenCalledTimes(1);
  expect(mocks.download).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(4 * 60 * 60 * 1000);
  expect(mocks.check).toHaveBeenCalledTimes(2);
  app.settings.autoCheckUpdates = false;
  await nextTick();
  await vi.advanceTimersByTimeAsync(4 * 60 * 60 * 1000);
  expect(mocks.check).toHaveBeenCalledTimes(2);
  wrapper.unmount();
  await vi.advanceTimersByTimeAsync(4 * 60 * 60 * 1000);
  expect(mocks.check).toHaveBeenCalledTimes(2);
});
it('automatically checks at startup with a saved opt-in and displays an app-wide notice', async () => {
  const app = useApp();
  app.settings.autoCheckUpdates = true;
  const wrapper = mount(AppUpdates, { global: { stubs: { ElDialog: true } } });
  wrappers.push(wrapper);
  await flushPromises();
  expect(mocks.check).not.toHaveBeenCalled();
  app.ready = true;
  await flushPromises();
  expect(mocks.check).toHaveBeenCalledTimes(1);
  expect(wrapper.text()).toContain('发现新版本');
  await wrapper.get('[aria-label="稍后处理更新"]').trigger('click');
  expect(wrapper.find('aside').exists()).toBe(false);
});
it('persists the automatic check switch immediately and restores it if saving fails', async () => {
  const wrapper = panel();
  await flushPromises();
  await wrapper.get('[aria-label="自动检查更新"]').trigger('click');
  await flushPromises();
  expect(mocks.api).toHaveBeenCalledWith(
    'save_settings',
    expect.objectContaining({ autoCheckUpdates: true }),
  );
  mocks.api.mockRejectedValueOnce(new Error('保存失败'));
  await wrapper.get('[aria-label="自动检查更新"]').trigger('click');
  await flushPromises();
  expect(useApp().settings.autoCheckUpdates).toBe(true);
  expect(wrapper.get('[role="alert"]').text()).toContain('保存失败');
});
it('renders manual check, download and install controls, with no automatic installation', async () => {
  const wrapper = panel();
  await flushPromises();
  await wrapper.get('.setting-row:nth-of-type(2) button').trigger('click');
  await flushPromises();
  expect(wrapper.text()).toContain('旅图 v1.1.0');
  await wrapper.get('.update-release button').trigger('click');
  await flushPromises();
  expect(wrapper.text()).toContain('签名已验证');
  expect(mocks.install).not.toHaveBeenCalled();
  await wrapper.get('.update-release button').trigger('click');
  await flushPromises();
  expect(mocks.confirm).toHaveBeenCalled();
  expect(mocks.install).toHaveBeenCalledTimes(1);
});
it('disables installation while jobs or review saves are unfinished and respects cancel', async () => {
  await downloaded();
  const wrapper = panel();
  await flushPromises();
  useLibrary().pending = 1;
  await nextTick();
  expect(wrapper.get('.update-release button').attributes('disabled')).toBeDefined();
  useLibrary().pending = 0;
  useApp().jobs = [{ id: 'job', status: 'paused' } as never];
  await nextTick();
  expect(wrapper.get('.update-release button').attributes('disabled')).toBeDefined();
  useApp().jobs = [];
  await nextTick();
  mocks.confirm.mockRejectedValueOnce('cancel');
  await wrapper.get('.update-release button').trigger('click');
  await flushPromises();
  expect(mocks.install).not.toHaveBeenCalled();
});
