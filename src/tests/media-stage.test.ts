import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import MediaStage from '../components/MediaStage.vue';
import { clearPreviewCache } from '../composables/preview';
import { createMemoryHistory, createRouter } from 'vue-router';
import type { AssetDetail } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({
  api: invoke,
  resourceUrl: (url: string) => url,
  errorMessage: (e: Error) => e.message,
}));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));
function video(id = 'a'): AssetDetail {
  return {
    ...(structuredClone(golden.assetDetail) as AssetDetail),
    id,
    mediaKind: 'video',
    filename: `${id}.mkv`,
  };
}
function photo(id = 'a'): AssetDetail {
  return { ...video(id), mediaKind: 'image', filename: `${id}.jpg` };
}
function pending<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}
beforeEach(() => {
  setActivePinia(createPinia());
  clearPreviewCache();
  invoke.mockReset();
});
it('shows the current thumbnail while its screen preview is slow, then upgrades the same photo', async () => {
  const screen = pending<unknown>();
  invoke.mockImplementation(async (command: string, request: { variant: string }) => {
    if (command !== 'resolve_preview') throw new Error(`unexpected ${command}`);
    return request.variant === 'screen' ? screen.promise : { ready: true, resourceUrl: 'small-a.jpg' };
  });
  const wrapper = mount(MediaStage, { props: { asset: photo() } });
  await flushPromises();
  expect(wrapper.get('img[alt="a.jpg"]').attributes('src')).toBe('small-a.jpg');
  expect(wrapper.text()).toContain('正在加载清晰预览');
  screen.resolve({ ready: true, resourceUrl: 'screen-a.jpg' });
  await flushPromises();
  expect(wrapper.get('img[alt="a.jpg"]').attributes('src')).toBe('screen-a.jpg');
  expect(wrapper.text()).not.toContain('正在加载清晰预览');
  wrapper.unmount();
});
it('never labels a late previous thumbnail or screen response as the new photo', async () => {
  const oldSmall = pending<unknown>(),
    oldScreen = pending<unknown>();
  invoke.mockImplementation(async (command: string, request: { assetId: string; variant: string }) => {
    if (command !== 'resolve_preview') throw new Error(`unexpected ${command}`);
    if (request.assetId === 'a') return request.variant === 'small' ? oldSmall.promise : oldScreen.promise;
    return { ready: true, resourceUrl: `${request.variant}-b.jpg` };
  });
  const wrapper = mount(MediaStage, { props: { asset: photo() } });
  await flushPromises();
  await wrapper.setProps({ asset: photo('b') });
  await flushPromises();
  expect(wrapper.get('img[alt="b.jpg"]').attributes('src')).toBe('screen-b.jpg');
  oldSmall.resolve({ ready: true, resourceUrl: 'late-small-a.jpg' });
  oldScreen.resolve({ ready: true, resourceUrl: 'late-screen-a.jpg' });
  await flushPromises();
  expect(wrapper.get('img[alt="b.jpg"]').attributes('src')).toBe('screen-b.jpg');
  expect(wrapper.find('img[alt="a.jpg"]').exists()).toBe(false);
  wrapper.unmount();
});
it('can display the original at1:1 from a ready thumbnail while the screen preview is pending', async () => {
  const previews = pending<unknown>();
  invoke.mockImplementation(async (command: string, request: { variant?: string }) => {
    if (command === 'resolve_preview')
      return request.variant === 'small' ? { ready: true, resourceUrl: 'small-a.jpg' } : previews.promise;
    if (command === 'resolve_original') return { resourceUrl: 'original-a.jpg' };
    throw new Error(`unexpected ${command}`);
  });
  const wrapper = mount(MediaStage, { props: { asset: photo(), actualPixels: true } });
  await flushPromises();
  expect(wrapper.get('img[alt="a.jpg"]').attributes('src')).toBe('original-a.jpg');
  expect(wrapper.get('img').classes()).toContain('actual-pixels');
  wrapper.unmount();
  previews.resolve({ ready: false });
  await flushPromises();
});
it('shows source errors persistently and permits retry without duplicate pending playback requests', async () => {
  let attempts = 0;
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'resolve_preview') return { ready: true, resourceUrl: 'poster.jpg' };
    if (command === 'resolve_playback') {
      if (++attempts === 1) throw new Error('来源目录离线，请重新连接磁盘');
      return new Promise((resolve) => {
        finish = resolve;
      });
    }
    throw new Error(`unexpected ${command}`);
  });
  const wrapper = mount(MediaStage, { props: { asset: video() } });
  await flushPromises();
  await wrapper.get('[aria-label="尝试内嵌播放"]').trigger('click');
  await flushPromises();
  expect(wrapper.get('[role="status"]').text()).toContain('来源目录离线');
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '重试播放')!
    .trigger('click');
  expect(wrapper.get<HTMLButtonElement>('[aria-label="正在准备播放"]').element.disabled).toBe(true);
  await wrapper.get('[aria-label="正在准备播放"]').trigger('click');
  expect(attempts).toBe(2);
  finish({ resourceUrl: 'video-a' });
  await flushPromises();
  expect(wrapper.get('video').attributes('src')).toBe('video-a');
  expect(wrapper.find('[role="status"]').exists()).toBe(false);
  wrapper.unmount();
});
it('offers a system player after an actual media element error', async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === 'resolve_preview') return { ready: true, resourceUrl: 'poster.jpg' };
    if (command === 'resolve_playback') return { resourceUrl: 'unsupported-video' };
    if (command === 'open_asset') return { opened: true };
    throw new Error(`unexpected ${command}`);
  });
  const wrapper = mount(MediaStage, { props: { asset: video() } });
  await flushPromises();
  await wrapper.get('[aria-label="尝试内嵌播放"]').trigger('click');
  await flushPromises();
  await wrapper.get('video').trigger('error');
  expect(wrapper.find('video').exists()).toBe(false);
  expect(wrapper.get('img').attributes('src')).toBe('poster.jpg');
  expect(wrapper.get('[role="status"]').text()).toContain('内嵌播放失败');
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '使用系统应用打开')!
    .trigger('click');
  expect(invoke).toHaveBeenCalledWith('open_asset', { assetId: 'a' });
  wrapper.unmount();
});
it('discards a late playback response even when navigating A to B to A', async () => {
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'resolve_preview') return { ready: true, resourceUrl: 'poster.jpg' };
    return new Promise((resolve) => {
      finish = resolve;
    });
  });
  const wrapper = mount(MediaStage, { props: { asset: video() } });
  await flushPromises();
  await wrapper.get('[aria-label="尝试内嵌播放"]').trigger('click');
  await wrapper.setProps({ asset: video('b') });
  await wrapper.setProps({ asset: video('a') });
  finish({ resourceUrl: 'obsolete-a' });
  await flushPromises();
  expect(wrapper.find('video').exists()).toBe(false);
  expect(wrapper.find('[aria-label="尝试内嵌播放"]').exists()).toBe(true);
  wrapper.unmount();
});
it('allows repeated deliberate preview retries, prevents another click while loading, and recovers', async () => {
  let attempt = 0;
  const slow = pending<unknown>();
  invoke.mockImplementation(async (command: string, request: { variant: string }) => {
    if (command !== 'resolve_preview') throw new Error(`unexpected ${command}`);
    if (request.variant === 'screen') attempt++;
    if (attempt === 2) return slow.promise;
    if (attempt < 4) return { ready: false, resourceUrl: null, reason: '媒体损坏' };
    return { ready: true, resourceUrl: `${request.variant}-recovered.jpg` };
  });
  const wrapper = mount(MediaStage, { props: { asset: photo() } });
  try {
    await flushPromises();
    const retry = () => wrapper.findAll('button').find((b) => b.text() === '重试预览')!;
    expect(wrapper.text()).toContain('媒体损坏');
    await retry().trigger('click');
    await flushPromises();
    expect(wrapper.text()).toContain('正在读取预览');
    expect(retry()).toBeUndefined();
    slow.resolve({ ready: false, resourceUrl: null, reason: '仍然无法解码' });
    await flushPromises();
    expect(wrapper.text()).toContain('仍然无法解码');
    await retry().trigger('click');
    await flushPromises();
    expect(wrapper.text()).toContain('媒体损坏');
    await retry().trigger('click');
    await flushPromises();
    expect(wrapper.get('img[alt="a.jpg"]').attributes('src')).toBe('screen-recovered.jpg');
    expect(attempt).toBe(4);
  } finally {
    wrapper.unmount();
  }
});
it('opens the failed media folder by its catalog ID and displays OS errors', async () => {
  const { ElMessage } = await import('element-plus');
  invoke.mockImplementation(async (command: string) => {
    if (command === 'resolve_preview') return { ready: false, resourceUrl: null, reason: '媒体损坏' };
    if (command === 'open_asset_location') throw new Error('系统无法打开所在文件夹');
    throw new Error(`unexpected ${command}`);
  });
  const wrapper = mount(MediaStage, { props: { asset: photo() } });
  try {
    await flushPromises();
    await wrapper
      .findAll('button')
      .find((b) => b.text() === '打开所在文件夹')!
      .trigger('click');
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith('open_asset_location', { assetId: 'a' });
    expect(ElMessage.error).toHaveBeenCalledWith('系统无法打开所在文件夹');
    expect(wrapper.text()).toContain('媒体损坏');
  } finally {
    wrapper.unmount();
  }
});
it('keeps cached missing-media previews and provides the source relink entry', async () => {
  invoke.mockImplementation(async () => ({ ready: true, resourceUrl: 'cached.jpg' }));
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/', component: {} },
      { path: '/sources', component: {} },
    ],
  });
  const wrapper = mount(MediaStage, {
    props: { asset: { ...photo(), fileState: 'missing' } },
    global: { plugins: [router] },
  });
  try {
    await flushPromises();
    expect(wrapper.get('img').attributes('src')).toBe('cached.jpg');
    expect(wrapper.text()).toContain('原文件缺失，已有预览和整理记录保留');
    const link = wrapper.get('a');
    expect(link.text()).toBe('重新定位来源');
    await link.trigger('click');
    await flushPromises();
    expect(router.currentRoute.value.path).toBe('/sources');
  } finally {
    wrapper.unmount();
  }
});
it('discards the old photo retry result after navigating to another photo', async () => {
  const retry = pending<unknown>();
  let retrying = false;
  invoke.mockImplementation(async (command: string, request: { assetId: string; variant: string }) => {
    if (command !== 'resolve_preview') throw new Error(`unexpected ${command}`);
    if (request.assetId === 'a')
      return retrying ? retry.promise : { ready: false, resourceUrl: null, reason: '媒体损坏' };
    return { ready: true, resourceUrl: `${request.variant}-b.jpg` };
  });
  const wrapper = mount(MediaStage, { props: { asset: photo() } });
  try {
    await flushPromises();
    retrying = true;
    await wrapper
      .findAll('button')
      .find((b) => b.text() === '重试预览')!
      .trigger('click');
    await flushPromises();
    await wrapper.setProps({ asset: photo('b') });
    await flushPromises();
    expect(wrapper.get('img').attributes('src')).toBe('screen-b.jpg');
    retry.resolve({ ready: true, resourceUrl: 'obsolete-retry-a.jpg' });
    await flushPromises();
    expect(wrapper.get('img').attributes('src')).toBe('screen-b.jpg');
    expect(wrapper.get('img').attributes('alt')).toBe('b.jpg');
  } finally {
    wrapper.unmount();
  }
});
