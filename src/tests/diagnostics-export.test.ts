import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import SettingsPage from '../pages/SettingsPage.vue';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, isDemo: false, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({
  ElMessage: { error: vi.fn(), success: vi.fn() },
  ElMessageBox: { confirm: vi.fn() },
}));
vi.mock('../components/ThirdPartyNotices.vue', () => ({ default: { template: '<span/>' } }));
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
  invoke.mockImplementation(async (name: string) => {
    if (name === 'cache_stats') return { bytes: 0, count: 0 };
    if (name === 'get_app_status') return { catalogVersion: 9 };
    if (name === 'pick_directory') return { path: '/owned/diagnostics' };
    if (name === 'export_diagnostics') return { fileName: 'travel-gallery-diagnostics-owned.json' };
    throw new Error(name);
  });
});
function page() {
  return mount(SettingsPage, { global: { stubs: { Icon: true, ElSwitch: true } } });
}
it('uses the authorized native writer for diagnostics instead of a browser download', async () => {
  const wrapper = page();
  await flushPromises();
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '导出诊断')!
    .trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith('pick_directory', { purpose: 'diagnostics' });
  expect(invoke).toHaveBeenCalledWith('export_diagnostics', { destinationPath: '/owned/diagnostics' });
  wrapper.unmount();
});
it('does not write a report when the native directory picker is canceled', async () => {
  const wrapper = page();
  await flushPromises();
  invoke.mockImplementation(async () => ({ path: null }));
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '导出诊断')!
    .trigger('click');
  await flushPromises();
  expect(invoke.mock.calls.filter(([name]) => name === 'export_diagnostics')).toHaveLength(0);
  wrapper.unmount();
});
