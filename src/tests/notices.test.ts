import { afterEach, expect, it, vi } from 'vitest';
import { flushPromises, mount } from '@vue/test-utils';
import ThirdPartyNotices from '../components/ThirdPartyNotices.vue';
const mounted: { unmount(): void }[] = [];
afterEach(() => {
  mounted.splice(0).forEach((w) => w.unmount());
  vi.unstubAllGlobals();
});
function setup() {
  const wrapper = mount(ThirdPartyNotices, {
    global: {
      stubs: { ElDialog: { props: ['modelValue'], template: '<div v-if="modelValue"><slot /></div>' } },
    },
  });
  mounted.push(wrapper);
  return wrapper;
}
it('reads only the packaged same-origin notice file and renders its contents as text', async () => {
  const fetcher = vi
    .fn()
    .mockResolvedValue({ ok: true, text: async () => 'MIT License\n<script>not executable</script>' });
  vi.stubGlobal('fetch', fetcher);
  const wrapper = setup();
  expect(fetcher).not.toHaveBeenCalled();
  await wrapper.get('button').trigger('click');
  await flushPromises();
  const url = fetcher.mock.calls[0]![0] as URL;
  expect(url.origin).toBe(window.location.origin);
  expect(url.pathname).toBe('/third-party-notices.txt');
  expect(wrapper.get('pre').text()).toContain('<script>not executable</script>');
  expect(wrapper.find('script').exists()).toBe(false);
  await wrapper.get('button').trigger('click');
  expect(fetcher).toHaveBeenCalledTimes(1);
});
it('keeps a failed resource read visible and retries the local file', async () => {
  const fetcher = vi
    .fn()
    .mockResolvedValueOnce({ ok: false })
    .mockResolvedValueOnce({ ok: true, text: async () => 'Recovered license' });
  vi.stubGlobal('fetch', fetcher);
  const wrapper = setup();
  await wrapper.get('button').trigger('click');
  await flushPromises();
  expect(wrapper.get('[role="alert"]').text()).toContain('许可文件暂时无法读取');
  expect(wrapper.find('pre').exists()).toBe(false);
  await wrapper.get('[role="alert"] button').trigger('click');
  await flushPromises();
  expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  expect(wrapper.get('pre').text()).toBe('Recovered license');
});
