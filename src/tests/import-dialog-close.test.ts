import { flushPromises, mount } from '@vue/test-utils';
import { ElDialog } from 'element-plus';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { createMemoryHistory, createRouter } from 'vue-router';
import { expect, it, vi } from 'vitest';
import ImportDialog from '../components/ImportDialog.vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { hasActiveModal } from '../composables/keyboard';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (error: Error) => error.message }));

it.each(['取消', 'Escape', '标题栏关闭', '开始索引'])(
  'closes the source dialog without waiting for an animation frame: %s',
  async (action) => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const app = useApp();
    vi.spyOn(app, 'refresh').mockResolvedValue();
    vi.spyOn(useLibrary(), 'search').mockResolvedValue();
    invoke.mockReset().mockImplementation(async (command: string) => {
      if (command === 'pick_directory') return { path: '/owned-fixture' };
      if (command === 'add_source') return { sourceId: 'new-source' };
      throw new Error(command);
    });
    const router = createRouter({
      history: createMemoryHistory(),
      routes: ['/welcome', '/library'].map((path) => ({ path, component: { render: () => null } })),
    });
    await router.push('/welcome');
    const frames = vi.spyOn(window, 'requestAnimationFrame').mockReturnValue(1);
    const wrapper = mount(ImportDialog, {
      attachTo: document.body,
      global: { plugins: [pinia, router], components: { ElDialog }, stubs: { transition: false } },
    });
    try {
      app.importOpen = true;
      await flushPromises();
      expect(hasActiveModal()).toBe(true);
      if (action === '开始索引') {
        await wrapper.get('.folder-picker').trigger('click');
        await flushPromises();
        await wrapper.get('button.primary').trigger('click');
      } else if (action === 'Escape') {
        await wrapper.get('[role="dialog"]').trigger('keydown', { key: 'Escape' });
      } else {
        const button = action === '取消' ? 'button.btn:not(.primary)' : 'button.el-dialog__headerbtn';
        await wrapper.get(button).trigger('click');
      }
      await flushPromises();
      expect(hasActiveModal()).toBe(false);
      expect(router.currentRoute.value.path).toBe(action === '开始索引' ? '/library' : '/welcome');
      expect(invoke.mock.calls.map(([name]) => name)).toEqual(
        action === '开始索引' ? ['pick_directory', 'add_source'] : [],
      );
      app.importOpen = true;
      await nextTick();
      expect(hasActiveModal()).toBe(true);
    } finally {
      wrapper.unmount();
      frames.mockRestore();
    }
  },
);

it('shows an updated overlap warning and only submits its confirmation on an explicit click', async () => {
  const pinia = createPinia();
  setActivePinia(pinia);
  const app = useApp();
  vi.spyOn(app, 'refresh').mockResolvedValue();
  vi.spyOn(useLibrary(), 'search').mockResolvedValue();
  const add = vi
    .fn()
    .mockRejectedValueOnce({
      code: 'SOURCE_OVERLAP',
      details: { sourceCount: 1, indexedCount: 2, overlapConfirmation: 'first' },
    })
    .mockRejectedValueOnce({
      code: 'SOURCE_OVERLAP',
      details: { sourceCount: 1, indexedCount: 3, overlapConfirmation: 'changed' },
    })
    .mockResolvedValue({ sourceId: 'new-source' });
  invoke.mockReset().mockImplementation(async (command: string, request: unknown) => {
    if (command === 'pick_directory') return { path: '/owned-fixture' };
    if (command === 'add_source') return add(request);
    throw new Error(command);
  });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/welcome', '/library'].map((path) => ({ path, component: { render: () => null } })),
  });
  await router.push('/welcome');
  const wrapper = mount(ImportDialog, {
    attachTo: document.body,
    global: { plugins: [pinia, router], components: { ElDialog } },
  });
  const button = (name: string) => wrapper.findAll('button').find((item) => item.text() === name)!;
  const submit = async (name: string) => {
    await button(name).trigger('click');
    await flushPromises();
  };
  try {
    app.importOpen = true;
    await nextTick();
    await wrapper.get('.folder-picker').trigger('click');
    await flushPromises();
    await submit('开始索引');
    expect(add).toHaveBeenCalledTimes(1);
    expect(add).toHaveBeenLastCalledWith(expect.objectContaining({ overlapConfirmation: undefined }));
    expect(wrapper.get('[role="alert"]').text()).toContain('已有 2 条索引记录');
    expect(button('开始索引').attributes()).toHaveProperty('disabled');
    await submit('开始索引');
    expect(add).toHaveBeenCalledTimes(1);
    await submit('确认并开始索引');
    expect(add).toHaveBeenCalledTimes(2);
    expect(add).toHaveBeenLastCalledWith(expect.objectContaining({ overlapConfirmation: 'first' }));
    expect(wrapper.get('[role="alert"]').text()).toContain('已有 3 条索引记录');
    expect(router.currentRoute.value.path).toBe('/welcome');
    await submit('确认并开始索引');
    expect(add).toHaveBeenCalledTimes(3);
    expect(add).toHaveBeenLastCalledWith(expect.objectContaining({ overlapConfirmation: 'changed' }));
    expect(hasActiveModal()).toBe(false);
    expect(router.currentRoute.value.path).toBe('/library');
  } finally {
    wrapper.unmount();
  }
});
