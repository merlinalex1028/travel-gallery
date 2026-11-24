import { setSelect } from './helpers/select';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { defineComponent, nextTick } from 'vue';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import MediaGrid from '../components/MediaGrid.vue';
import LibraryToolbar from '../features/LibraryToolbar.vue';
import { useLibrary } from '../stores/library';
import type { AssetSummary } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), error: vi.fn() } }));
const assets = Array.from(
  { length: 125 },
  (_, i) =>
    ({
      ...golden.assetDetail,
      id: `asset-${i}`,
    }) as AssetSummary,
);
let wrapper: VueWrapper | undefined;
beforeEach(() => {
  document.body.innerHTML = '';
  setActivePinia(createPinia());
  vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockReturnValue(1200);
  vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(700);
  invoke.mockReset();
  invoke.mockImplementation(async (name: string, request: { cursor?: string }) => {
    if (name !== 'query_assets') throw new Error(name);
    return {
      items: request.cursor ? assets.slice(120) : assets.slice(0, 120),
      total: 125,
      nextCursor: request.cursor ? null : 'next-page',
      queryFingerprint: 'query',
    };
  });
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  vi.restoreAllMocks();
  document.body.innerHTML = '';
});
async function openGrid(initialScroll = 0) {
  const lib = useLibrary();
  lib.items = assets.slice(0, 120);
  lib.total = 125;
  lib.cursor = 'next-page';
  lib.scrollTop = initialScroll;
  wrapper = mount(
    defineComponent({
      components: { MediaGrid, LibraryToolbar },
      setup: () => ({ lib }),
      template: `<LibraryToolbar />
      <MediaGrid :items="lib.items" :density="220" :has-more="!!lib.cursor"
        :loading-more="lib.loadingMore" :is-selected="lib.isSelected"
        :initial-scroll="lib.scrollTop" group-by="none" @scroll="lib.scrollTop = $event" />`,
    }),
    {
      attachTo: document.body,
      global: { stubs: { Icon: true, MediaCard: true } },
    },
  );
  await flushPromises();
  return lib;
}
async function scrollTo(top: number) {
  const viewport = wrapper!.get<HTMLElement>('.media-viewport');
  viewport.element.scrollTop = top;
  await viewport.trigger('scroll');
  expect(useLibrary().scrollTop).toBe(top);
}
function offset() {
  return wrapper!.get<HTMLElement>('.media-viewport').element.scrollTop;
}

it.each(['field', 'direction', 'kind', 'linked'] as const)(
  'returns the live grid to its head when the toolbar changes %s',
  async (change) => {
    await openGrid();
    await scrollTo(3000);
    if (change === 'field') await setSelect(wrapper!, '排序字段', 'rating');
    if (change === 'direction') await wrapper!.get('[aria-label="改为升序"]').trigger('click');
    if (change === 'kind')
      await wrapper!
        .findAll('button')
        .find((b) => b.text() === '照片')!
        .trigger('click');
    if (change === 'linked') await wrapper!.get('input[type="checkbox"]').setValue(false);
    await flushPromises();
    expect(offset()).toBe(0);
    expect(useLibrary().scrollTop).toBe(0);
  },
);
it('applies a source change and a later nested filter edit to the mounted viewport', async () => {
  const lib = await openGrid();
  await scrollTo(3000);
  lib.setFilter({ sourceIds: ['one'] });
  await nextTick();
  expect(offset()).toBe(0);
  await scrollTo(2100);
  lib.filter.sourceIds!.push('two');
  await nextTick();
  expect(offset()).toBe(0);
});
it('preserves the scroll anchor on same-query refresh and appended pages', async () => {
  const lib = await openGrid();
  await scrollTo(2100);
  await lib.search();
  await flushPromises();
  expect(offset()).toBe(2100);
  await lib.search(true);
  await flushPromises();
  expect(lib.items).toHaveLength(125);
  expect(offset()).toBe(2100);
});
it('restores an explicit return anchor after changing back to the saved query', async () => {
  const lib = await openGrid(3000);
  expect(offset()).toBe(3000);
  lib.setFilter({ keyword: 'temporary scope' });
  await nextTick();
  lib.setFilter({});
  lib.sort.direction = 'asc';
  lib.scrollTop = 3000;
  await nextTick();
  expect(offset()).toBe(3000);
  expect(lib.scrollTop).toBe(3000);
});
it('resets the grid without changing a frozen target or its exclusions', async () => {
  const lib = await openGrid();
  lib.snapshot = { id: 'frozen', count: 125, createdAt: 1, expiresAt: 100, queryFingerprint: 'old' };
  lib.excluded = new Set(['asset-1']);
  const target = lib.target;
  await scrollTo(3000);
  lib.filter.ratingMin = 3;
  await nextTick();
  expect(offset()).toBe(0);
  expect(lib.target).toEqual(target);
  expect(lib.count).toBe(124);
});
