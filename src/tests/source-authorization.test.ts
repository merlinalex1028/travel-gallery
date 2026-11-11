import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import { expect, it, vi } from 'vitest';
import SourceCard from '../features/SourceCard.vue';
import type { SourceRoot } from '../services/contracts';
import { useApp } from '../stores/app';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (error: Error) => error.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));

it('offers a direct native reauthorization for the existing source and preserves two-stage confirmation', async () => {
  setActivePinia(createPinia());
  vi.spyOn(useApp(), 'refresh').mockResolvedValue();
  invoke.mockImplementation(async (name: string) => {
    if (name === 'pick_directory') return { path: '/source' };
    if (name === 'preview_relink_source')
      return {
        newPath: '/source',
        matched: 5,
        conflicts: 0,
        missing: 0,
        sampleChecks: [],
        verificationToken: 'token',
      };
    if (name === 'commit_relink_source') return { updated: true };
    throw new Error(name);
  });
  const source: SourceRoot = {
    id: 'source-id',
    displayName: '相册',
    absolutePath: '/source',
    volumeId: null,
    isEnabled: true,
    isOnline: false,
    lastScanAt: null,
    totalAssets: 5,
    offlineAssets: 5,
  };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/', component: { template: '<div />' } }],
  });
  await router.push('/');
  const wrapper = mount(SourceCard, {
    props: { source },
    global: {
      plugins: [router],
      stubs: {
        ElDropdown: true,
        ElDropdownItem: true,
        ElDropdownMenu: true,
        ElDialog: {
          props: ['modelValue'],
          template: '<section v-if="modelValue"><slot /><slot name="footer" /></section>',
        },
      },
    },
  });
  try {
    const authorize = () => wrapper.findAll('button').find((button) => button.text() === '重新授权来源');
    expect(wrapper.text()).toContain('来源离线');
    expect(authorize()).toBeUndefined();
    await wrapper.setProps({ source: { ...source, requiresAuthorization: true } });
    expect(wrapper.text()).toContain('需要重新授权');
    await authorize()!.trigger('click');
    await flushPromises();
    expect(invoke.mock.calls).toEqual([
      ['pick_directory', { purpose: 'relink' }],
      ['preview_relink_source', { sourceId: 'source-id', selectedNewPath: '/source' }],
    ]);
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '确认更新来源路径')!
      .trigger('click');
    await flushPromises();
    expect(invoke).toHaveBeenLastCalledWith('commit_relink_source', {
      sourceId: 'source-id',
      verificationToken: 'token',
    });
    await wrapper.setProps({ source: { ...source, isOnline: true, offlineAssets: 0 } });
    expect(wrapper.text()).toContain('在线');
    expect(authorize()).toBeUndefined();
  } finally {
    wrapper.unmount();
  }
});
