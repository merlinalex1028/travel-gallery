import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { flushPromises, mount } from '@vue/test-utils';
import TagsPage from '../pages/TagsPage.vue';
import TripsPage from '../pages/TripsPage.vue';
import { useApp } from '../stores/app';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (error: Error) => error.message }));
vi.mock('element-plus', () => ({
  ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
  ElMessageBox: { confirm: vi.fn().mockRejectedValue(new Error('cancel')) },
}));
type Page = 'tags' | 'trips';
const pages: Page[] = ['tags', 'trips'];
function pending<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}
beforeEach(() => {
  invoke.mockReset();
});
async function setup(page: Page) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const app = useApp(),
    refresh = vi.spyOn(app, 'refresh').mockResolvedValue();
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/tags', '/trips', '/trips/:id', '/filter'].map((path) => ({
      path,
      component: { render: () => null },
    })),
  });
  await router.push(`/${page}`);
  await router.isReady();
  const wrapper = mount(page === 'tags' ? TagsPage : TripsPage, {
    global: {
      plugins: [pinia, router],
      stubs: {
        MediaThumbnail: true,
        ElDropdown: true,
        ElDropdownMenu: true,
        ElDropdownItem: true,
        ElDialog: {
          props: ['modelValue', 'title'],
          template: '<div v-if="modelValue"><h2>{{ title }}</h2><slot/><slot name="footer"/></div>',
        },
      },
    },
  });
  const create = async () => {
    await wrapper
      .findAll('button')
      .find((button) => button.text() === (page === 'tags' ? '新建标签' : '新建旅行'))!
      .trigger('click');
    await flushPromises();
  };
  await create();
  const input = () => wrapper.get('form input[maxlength="100"]');
  await input().setValue('固定的新建草稿');
  const submit = () => wrapper.get('form').trigger('submit');
  const response = (id: string) => (page === 'tags' ? { tagId: id } : { tripId: id });
  return { wrapper, refresh, input, submit, create, response };
}

it.each(pages)('%s prevents a second form submission before the DOM becomes disabled', async (page) => {
  const waiting = pending<unknown>();
  invoke.mockImplementation(() => waiting.promise);
  const h = await setup(page);
  try {
    const form = h.wrapper.get('form').element;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(h.input().attributes('disabled')).toBeDefined();
    waiting.resolve(h.response(invoke.mock.calls[0]![1].id));
    await flushPromises();
    expect(h.refresh).toHaveBeenCalledTimes(1);
    expect(h.wrapper.find('form').exists()).toBe(false);
  } finally {
    h.wrapper.unmount();
  }
});
it.each(pages)('%s retries only the list read after a confirmed commit and failed refresh', async (page) => {
  const h = await setup(page);
  h.refresh.mockRejectedValueOnce(new Error('controlled refresh failure'));
  invoke.mockImplementation(async (_command: string, request: { id: string }) => h.response(request.id));
  try {
    await h.submit();
    await flushPromises();
    expect(h.wrapper.get('[role="alert"]').text()).toContain('已保存，列表刷新失败');
    expect(h.input().element).toHaveProperty('value', '固定的新建草稿');
    await h.submit();
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(h.refresh).toHaveBeenCalledTimes(2);
    expect(h.wrapper.find('form').exists()).toBe(false);
  } finally {
    h.wrapper.unmount();
  }
});
it.each(pages)('%s retries an unconfirmed write using the same resource ID', async (page) => {
  const objects = new Map<string, string>();
  const h = await setup(page);
  invoke.mockImplementation(async (_command: string, request: { id: string; name: string }) => {
    objects.set(request.id, request.name);
    if (invoke.mock.calls.length === 1) throw new Error('write response lost after commit');
    return h.response(request.id);
  });
  try {
    await h.submit();
    await flushPromises();
    expect(h.wrapper.get('[role="alert"]').text()).toContain('write response lost');
    expect(h.refresh).not.toHaveBeenCalled();
    await h.submit();
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(2);
    const ids = invoke.mock.calls.map(([, request]) => request.id);
    expect(ids[0]).toEqual(expect.any(String));
    expect(ids[1]).toBe(ids[0]);
    expect(objects.size).toBe(1);
    expect(h.wrapper.find('form').exists()).toBe(false);
  } finally {
    h.wrapper.unmount();
  }
});
it.each(pages)(
  '%s updates the same object when the user edits a committed draft after refresh failure',
  async (page) => {
    const h = await setup(page);
    h.refresh.mockRejectedValueOnce(new Error('refresh unavailable'));
    invoke.mockImplementation(async (_command: string, request: { id: string }) => h.response(request.id));
    try {
      await h.submit();
      await flushPromises();
      await h.input().setValue('修改同一份草稿');
      await h.submit();
      await flushPromises();
      expect(invoke).toHaveBeenCalledTimes(2);
      expect(invoke.mock.calls[1]![1]).toMatchObject({
        id: invoke.mock.calls[0]![1].id,
        name: '修改同一份草稿',
      });
      expect(h.wrapper.find('form').exists()).toBe(false);
    } finally {
      h.wrapper.unmount();
    }
  },
);
it.each(pages)('%s allocates a new ID only when the user starts another draft', async (page) => {
  const h = await setup(page);
  invoke.mockImplementation(async (_command: string, request: { id: string }) => h.response(request.id));
  try {
    await h.submit();
    await flushPromises();
    await h.create();
    await h.input().setValue('另一份新建草稿');
    await h.submit();
    await flushPromises();
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke.mock.calls[1]![1].id).not.toBe(invoke.mock.calls[0]![1].id);
  } finally {
    h.wrapper.unmount();
  }
});
it.each(pages)('%s does not send a whitespace-only draft', async (page) => {
  const h = await setup(page);
  try {
    await h.input().setValue('   ');
    await h.submit();
    await flushPromises();
    expect(invoke).not.toHaveBeenCalled();
    expect(h.refresh).not.toHaveBeenCalled();
  } finally {
    h.wrapper.unmount();
  }
});
