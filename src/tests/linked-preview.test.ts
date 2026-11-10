import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { defineComponent } from 'vue';
import { flushPromises, mount } from '@vue/test-utils';
import Inspector from '../components/Inspector.vue';
import { useLibrary } from '../stores/library';
import { clearPreviewCache } from '../composables/preview';
import { photoKey } from '../composables/keyboard';
import type { AssetDetail } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({
  api: invoke,
  resourceUrl: (url: string) => url,
  errorMessage: (error: Error) => error.message,
}));
vi.mock('element-plus', async (importOriginal) => ({
  ...(await importOriginal<typeof import('element-plus')>()),
  ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() },
}));
const Dialog = defineComponent({
  props: ['modelValue'],
  emits: ['update:modelValue'],
  template: `<div v-if="modelValue" role="dialog" data-modal="true"><slot/><slot name="footer"/><button aria-label="关闭配套素材窗口" @click="$emit('update:modelValue',false)">关闭</button></div>`,
});
function asset(id: string, video = false): AssetDetail {
  return {
    ...(structuredClone(golden.assetDetail) as AssetDetail),
    id,
    filename: video ? `${id}.MOV` : `${id}.jpg`,
    extension: video ? 'mov' : 'jpg',
    mediaKind: video ? 'video' : 'image',
  };
}
const mounted: { unmount(): void }[] = [];
afterEach(() => {
  mounted.splice(0).forEach((wrapper) => wrapper.unmount());
  document.body.innerHTML = '';
});
beforeEach(() => {
  setActivePinia(createPinia());
  clearPreviewCache();
  invoke.mockReset();
});
function inspector(confirmed: boolean, playback: () => Promise<{ resourceUrl: string }>) {
  const lib = useLibrary();
  lib.detail = asset('still');
  invoke.mockImplementation(async (command: string, request: { assetId: string }) => {
    if (command === 'get_media_links')
      return request.assetId === 'still'
        ? [
            {
              id: 'pair',
              primaryAssetId: 'still',
              secondaryAssetId: 'motion',
              linkType: 'live_photo',
              matchMethod: confirmed ? 'apple_content_identifier' : 'name_candidate_requires_confirmation',
              confidence: confirmed ? 0.99 : 0.3,
              confirmed,
            },
          ]
        : [];
    if (command === 'get_asset') return asset(request.assetId, true);
    if (command === 'resolve_preview') return { ready: true, resourceUrl: `${request.assetId}-poster` };
    if (command === 'resolve_playback') return playback();
    throw new Error(`Unexpected mutation or command ${command}`);
  });
  const wrapper = mount(Inspector, {
    attachTo: document.body,
    global: {
      stubs: {
        ElDialog: Dialog,
        MediaThumbnail: true,
        ReviewControls: true,
        RouterLink: true,
        QualityInspector: true,
      },
    },
  });
  mounted.push(wrapper);
  return { wrapper, lib };
}
it('plays a companion without changing the review target or confirming a candidate, and isolates keyboard scope', async () => {
  const { wrapper, lib } = inspector(false, async () => ({ resourceUrl: 'motion-playback' }));
  await flushPromises();
  await wrapper.get('[aria-label="播放配套文件 motion.MOV"]').trigger('click');
  await flushPromises();
  expect(lib.detail?.id).toBe('still');
  expect(photoKey(new KeyboardEvent('keydown', { key: 'p' }))).toBeNull();
  expect(invoke).not.toHaveBeenCalledWith('resolve_playback', expect.anything());
  await wrapper.get('[aria-label="尝试内嵌播放"]').trigger('click');
  await flushPromises();
  expect(wrapper.get('video').attributes('src')).toBe('motion-playback');
  expect(invoke).toHaveBeenCalledWith('resolve_playback', { assetId: 'motion' });
  expect(lib.detail?.id).toBe('still');
  expect(
    invoke.mock.calls.some(([command]) =>
      ['apply_review_batch', 'link_media', 'unlink_media'].includes(command),
    ),
  ).toBe(false);
  await wrapper.get('[aria-label="关闭配套素材窗口"]').trigger('click');
  expect(wrapper.find('video').exists()).toBe(false);
  expect(photoKey(new KeyboardEvent('keydown', { key: 'p' }))).toBe('p');
});
it('closes on a primary-asset change and discards a late companion playback response', async () => {
  let resolve!: (value: { resourceUrl: string }) => void;
  const { wrapper, lib } = inspector(
    true,
    () =>
      new Promise((finish) => {
        resolve = finish;
      }),
  );
  await flushPromises();
  await wrapper.get('[aria-label="播放配套文件 motion.MOV"]').trigger('click');
  await flushPromises();
  await wrapper.get('[aria-label="尝试内嵌播放"]').trigger('click');
  await flushPromises();
  lib.detail = asset('other');
  await flushPromises();
  expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  resolve({ resourceUrl: 'obsolete-motion' });
  await flushPromises();
  expect(wrapper.find('video').exists()).toBe(false);
  expect(lib.detail?.id).toBe('other');
});
