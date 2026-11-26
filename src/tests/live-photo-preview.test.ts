import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import MediaStage from '../components/MediaStage.vue';
import { clearPreviewCache } from '../composables/preview';
import type { AssetDetail, MediaLink } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../services/api')>()),
  api: invoke,
  resourceUrl: (url: string) => url,
}));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));

function photo(id = 'still', linked = true): AssetDetail {
  return {
    ...(structuredClone(golden.assetDetail) as AssetDetail),
    id,
    filename: `${id}.JPG`,
    mediaKind: 'image',
    linkedMediaType: linked ? 'live_photo' : undefined,
    linkedMediaCount: linked ? 1 : 0,
  };
}
const link: MediaLink = {
  id: 'pair',
  primaryAssetId: 'still',
  secondaryAssetId: 'motion',
  linkType: 'live_photo',
  matchMethod: 'apple_content_identifier',
  confidence: 0.99,
  confirmed: true,
};
function pending<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((finish) => {
    resolve = finish;
  });
  return { promise, resolve };
}
let links: () => Promise<MediaLink[]>;
let playback: () => Promise<{ resourceUrl: string }>;
const mounted: { unmount(): void }[] = [];
function stage(asset = photo()) {
  const wrapper = mount(MediaStage, {
    props: { asset },
    global: { stubs: { RouterLink: true } },
  });
  mounted.push(wrapper);
  return wrapper;
}
beforeEach(() => {
  setActivePinia(createPinia());
  clearPreviewCache();
  invoke.mockReset();
  links = async () => [link];
  playback = async () => ({ resourceUrl: 'motion.mov' });
  invoke.mockImplementation(async (command: string, request: { assetId: string }) => {
    if (command === 'resolve_preview') return { ready: true, resourceUrl: `${request.assetId}.jpg` };
    if (command === 'get_media_links') return links();
    if (command === 'resolve_playback') return playback();
    if (command === 'open_asset') return { opened: true };
    throw new Error(`Unexpected mutation or command ${command}`);
  });
});
afterEach(() => mounted.splice(0).forEach((wrapper) => wrapper.unmount()));

const badgeSelector = '[aria-label="Live Photo：悬停播放动态片段"]';

it('plays once per mouse visit, returns to the still at the end, and requires leaving before another hover play', async () => {
  const wrapper = stage();
  const badge = wrapper.get(badgeSelector);
  expect(badge.text()).toBe('LIVE');
  expect(badge.classes()).not.toContain('btn');
  await badge.trigger('pointerenter', { pointerType: 'mouse' });
  await flushPromises();
  expect(wrapper.get('video').element.muted).toBe(true);
  expect(wrapper.get('video').attributes('controls')).toBeUndefined();
  expect(wrapper.get('video').attributes('loop')).toBeUndefined();
  await wrapper.get('video').trigger('ended');
  expect(wrapper.find('video').exists()).toBe(false);
  await badge.trigger('pointermove', { pointerType: 'mouse' });
  await badge.trigger('pointerenter', { pointerType: 'mouse' });
  await badge.trigger('click', { detail: 1 });
  await flushPromises();
  expect(invoke.mock.calls.filter(([name]) => name === 'resolve_playback')).toHaveLength(1);
  await badge.trigger('pointerleave', { pointerType: 'mouse' });
  await badge.trigger('pointerenter', { pointerType: 'mouse' });
  await flushPromises();
  expect(invoke.mock.calls.filter(([name]) => name === 'resolve_playback')).toHaveLength(2);
});

it('stops immediately on pointer leave and releases the decoder', async () => {
  const wrapper = stage();
  const badge = wrapper.get(badgeSelector);
  await badge.trigger('pointerenter', { pointerType: 'mouse' });
  await flushPromises();
  const video = wrapper.get('video').element;
  const pause = vi.spyOn(video, 'pause');
  await badge.trigger('pointerleave', { pointerType: 'mouse' });
  expect(wrapper.find('video').exists()).toBe(false);
  expect(wrapper.get('img').attributes('src')).toBe('still.jpg');
  expect(pause).toHaveBeenCalledOnce();
  expect(video.hasAttribute('src')).toBe(false);
});

it('cancels a pending lookup when leaving, then only plays the new visit after reentry', async () => {
  const first = pending<MediaLink[]>();
  const second = pending<MediaLink[]>();
  let visits = 0;
  links = () => (++visits === 1 ? first.promise : second.promise);
  const wrapper = stage();
  const badge = wrapper.get(badgeSelector);
  await badge.trigger('pointerenter', { pointerType: 'mouse' });
  await badge.trigger('pointerleave', { pointerType: 'mouse' });
  await badge.trigger('pointerenter', { pointerType: 'mouse' });
  first.resolve([link]);
  await flushPromises();
  expect(invoke.mock.calls.some(([name]) => name === 'resolve_playback')).toBe(false);
  second.resolve([link]);
  await flushPromises();
  expect(invoke.mock.calls.filter(([name]) => name === 'resolve_playback')).toHaveLength(1);
  expect(wrapper.find('video').exists()).toBe(true);
});

it('discards a late playback response after leaving the badge', async () => {
  const response = pending<{ resourceUrl: string }>();
  playback = () => response.promise;
  const wrapper = stage();
  const badge = wrapper.get(badgeSelector);
  await badge.trigger('pointerenter', { pointerType: 'mouse' });
  await flushPromises();
  await badge.trigger('pointerleave', { pointerType: 'mouse' });
  response.resolve({ resourceUrl: 'late.mov' });
  await flushPromises();
  expect(wrapper.find('video').exists()).toBe(false);
  expect(badge.attributes('aria-busy')).toBe('false');
});

it('resets the mouse visit when navigating to another asset and back', async () => {
  const wrapper = stage();
  await wrapper.get(badgeSelector).trigger('pointerenter', { pointerType: 'mouse' });
  await flushPromises();
  await wrapper.setProps({ asset: photo('other', false) });
  await wrapper.setProps({ asset: photo() });
  await wrapper.get(badgeSelector).trigger('pointerenter', { pointerType: 'mouse' });
  await flushPromises();
  expect(invoke.mock.calls.filter(([name]) => name === 'resolve_playback')).toHaveLength(2);
  expect(wrapper.find('video').exists()).toBe(true);
});

it('keeps touch entry inert and supports deliberate tap and keyboard activation', async () => {
  const wrapper = stage();
  const badge = wrapper.get(badgeSelector);
  await badge.trigger('pointerenter', { pointerType: 'touch' });
  await flushPromises();
  expect(invoke.mock.calls.some(([name]) => name === 'get_media_links')).toBe(false);
  await badge.trigger('click', { detail: 1 });
  await flushPromises();
  expect(wrapper.find('video').exists()).toBe(true);
  await badge.trigger('click', { detail: 1 });
  expect(wrapper.find('video').exists()).toBe(false);
  await badge.trigger('click', { detail: 0 });
  await flushPromises();
  expect(wrapper.find('video').exists()).toBe(true);
  await badge.trigger('blur');
  expect(wrapper.find('video').exists()).toBe(false);
});

it('keeps playback failures actionable after leaving the badge', async () => {
  const wrapper = stage();
  const badge = wrapper.get(badgeSelector);
  await badge.trigger('pointerenter', { pointerType: 'mouse' });
  await flushPromises();
  await wrapper.get('video').trigger('error');
  await badge.trigger('pointerleave', { pointerType: 'mouse' });
  await badge.trigger('blur');
  expect(wrapper.get('[role="status"]').text()).toContain('内嵌播放失败');
  await wrapper
    .findAll('button')
    .find((button) => button.text() === '使用系统应用打开')!
    .trigger('click');
  expect(invoke).toHaveBeenCalledWith('open_asset', { assetId: 'motion' });
});

it('plays only on request, keeps the still as the review target, and returns to it when the clip ends', async () => {
  const asset = photo();
  const before = structuredClone(asset);
  const wrapper = stage(asset);
  await flushPromises();
  expect(invoke.mock.calls.every(([command]) => command === 'resolve_preview')).toBe(true);
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith('get_media_links', { assetId: 'still' });
  expect(invoke).toHaveBeenCalledWith('resolve_playback', { assetId: 'motion' });
  expect(wrapper.get('video').attributes('src')).toBe('motion.mov');
  await wrapper.get('video').trigger('ended');
  expect(wrapper.find('video').exists()).toBe(false);
  expect(wrapper.get('img').attributes('src')).toBe('still.jpg');
  expect(asset).toEqual(before);
  expect(invoke.mock.calls.map(([command]) => command)).toEqual([
    'resolve_preview',
    'resolve_preview',
    'get_media_links',
    'resolve_playback',
  ]);
});

it('can stop a dynamic clip and play it again', async () => {
  const wrapper = stage();
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  await flushPromises();
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  expect(wrapper.find('video').exists()).toBe(false);
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  await flushPromises();
  expect(wrapper.find('video').exists()).toBe(true);
});

it.each(['navigate', 'unmount'])(
  'releases the video decoder and source when leaving the preview (%s)',
  async (action) => {
    const wrapper = stage();
    await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
    await flushPromises();
    const video = wrapper.get('video').element;
    const pause = vi.spyOn(video, 'pause');
    const load = vi.spyOn(video, 'load');
    if (action === 'navigate') await wrapper.setProps({ asset: photo('other', false) });
    else {
      wrapper.unmount();
      mounted.splice(mounted.indexOf(wrapper), 1);
    }
    expect(pause).toHaveBeenCalledOnce();
    expect(video.hasAttribute('src')).toBe(false);
    expect(load).toHaveBeenCalledOnce();
  },
);

it('does not offer Live Photo playback for an ordinary still or an unconfirmed candidate', async () => {
  const wrapper = stage(photo('candidate', false));
  await flushPromises();
  expect(wrapper.find('[aria-label="Live Photo：悬停播放动态片段"]').exists()).toBe(false);
});

it.each([
  ['unconfirmed', [{ ...link, confirmed: false }]],
  ['reversed', [{ ...link, primaryAssetId: 'other' }]],
  ['wrong type', [{ ...link, linkType: 'raw_jpeg' }]],
  ['removed', []],
  ['ambiguous', [link, { ...link, id: 'second', secondaryAssetId: 'other-motion' }]],
])('rechecks links and never guesses a companion when the pair is %s', async (_, result) => {
  links = async () => result as MediaLink[];
  const wrapper = stage();
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  await flushPromises();
  expect(invoke.mock.calls.some(([command]) => command === 'resolve_playback')).toBe(false);
  expect(wrapper.find('video').exists()).toBe(false);
  expect(wrapper.get('[role="status"]').text()).toContain('配套素材');
});

it('opens the companion movie in the system player after an actual decoder failure', async () => {
  const wrapper = stage();
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  await flushPromises();
  await wrapper.get('video').trigger('error');
  expect(wrapper.get('img').attributes('src')).toBe('still.jpg');
  expect(wrapper.get('[role="status"]').text()).toContain('内嵌播放失败');
  await wrapper
    .findAll('button')
    .find((button) => button.text() === '使用系统应用打开')!
    .trigger('click');
  expect(invoke).toHaveBeenCalledWith('open_asset', { assetId: 'motion' });
});

it('keeps the still visible and permits retry after an offline playback error', async () => {
  playback = async () => {
    throw new Error('来源目录离线，请重新连接磁盘');
  };
  const wrapper = stage();
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  await flushPromises();
  expect(wrapper.get('img').attributes('src')).toBe('still.jpg');
  expect(wrapper.get('[role="status"]').text()).toContain('来源目录离线');
  playback = async () => ({ resourceUrl: 'reconnected.mov' });
  await wrapper
    .findAll('button')
    .find((button) => button.text() === '重试播放')!
    .trigger('click');
  await flushPromises();
  expect(wrapper.get('video').attributes('src')).toBe('reconnected.mov');
});

it('ignores a late link lookup after navigating A to B to A and prevents duplicate pending requests', async () => {
  const lookup = pending<MediaLink[]>();
  links = () => lookup.promise;
  const wrapper = stage();
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  expect(wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').attributes('aria-busy')).toBe('true');
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  expect(invoke.mock.calls.filter(([command]) => command === 'get_media_links')).toHaveLength(1);
  await wrapper.setProps({ asset: photo('other', false) });
  await wrapper.setProps({ asset: photo() });
  lookup.resolve([link]);
  await flushPromises();
  expect(invoke.mock.calls.some(([command]) => command === 'resolve_playback')).toBe(false);
  expect(wrapper.find('video').exists()).toBe(false);
});

it('discards a late playback response when the pair is removed from the same still', async () => {
  const response = pending<{ resourceUrl: string }>();
  playback = () => response.promise;
  const wrapper = stage();
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  await flushPromises();
  await wrapper.setProps({ asset: photo('still', false) });
  response.resolve({ resourceUrl: 'obsolete.mov' });
  await flushPromises();
  expect(wrapper.find('video').exists()).toBe(false);
  expect(wrapper.find('[aria-label="Live Photo：悬停播放动态片段"]').exists()).toBe(false);
});

it('does not resolve a companion after unmounting during a link lookup', async () => {
  const lookup = pending<MediaLink[]>();
  links = () => lookup.promise;
  const wrapper = stage();
  await wrapper.get('[aria-label="Live Photo：悬停播放动态片段"]').trigger('click');
  wrapper.unmount();
  mounted.splice(mounted.indexOf(wrapper), 1);
  lookup.resolve([link]);
  await flushPromises();
  expect(invoke.mock.calls.some(([command]) => command === 'resolve_playback')).toBe(false);
});
