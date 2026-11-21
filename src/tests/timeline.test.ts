import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import TimelinePage from '../pages/TimelinePage.vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import type { TimelineDay } from '../services/contracts';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));
const mounted: { unmount(): void }[] = [];
afterEach(() => mounted.splice(0).forEach((w) => w.unmount()));
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
  useApp().trips = [
    { id: 'trip', name: '旅行 A', description: '', startAt: null, endAt: null, coverAssetId: null, count: 3 },
  ];
});
const days: TimelineDay[] = [
  { date: '2026-06-15', count: 2, photos: 1, videos: 1, reviewed: 1, unknownTimezone: 1 },
  { date: null, count: 1, photos: 1, videos: 0, reviewed: 0, unknownTimezone: 0 },
];
async function setup() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/timeline', '/filter'].map((path) => ({ path, component: { render: () => null } })),
  });
  await router.push('/timeline');
  const wrapper = mount(TimelinePage, { global: { plugins: [router] } });
  mounted.push(wrapper);
  await flushPromises();
  return { wrapper, router, lib: useLibrary() };
}
it('excludes unknown dates from shooting-day totals and preserves trip scope when drilling into unknown dates', async () => {
  invoke.mockResolvedValue(days);
  const { wrapper, router, lib } = await setup();
  expect(wrapper.findAll('.timeline-summary strong').map((n) => n.text())).toEqual(['1', '3', '1', '1']);
  expect(wrapper.findAll('.timeline-day')[0]!.text()).toContain('1 项时区未知');
  expect(wrapper.findAll('.timeline-day')[0]!.text()).toContain('1 照片 · 1 视频');
  await wrapper.get('select').setValue('trip');
  await flushPromises();
  expect(invoke).toHaveBeenLastCalledWith('timeline_stats', { filter: { collectionId: 'trip' } });
  await wrapper.findAll('.timeline-day')[1]!.trigger('click');
  await flushPromises();
  expect(lib.filter).toEqual({ captureDateKnown: false, collectionId: 'trip' });
  expect(router.currentRoute.value.fullPath).toBe('/filter?showResults=1');
});
it('opens a reliable local date without implying that a known time zone comes from EXIF', async () => {
  invoke.mockResolvedValue([{ ...days[0]!, unknownTimezone: 0 }, days[1]]);
  const { wrapper, lib } = await setup();
  const day = wrapper.findAll('.timeline-day')[0]!;
  expect(day.text()).toContain('时区已知');
  await day.trigger('click');
  expect(lib.filter).toEqual({
    captureDateKnown: true,
    captureLocalFrom: '2026-06-15',
    captureLocalTo: '2026-06-15',
  });
});
it('keeps the selected travel range when an earlier aggregate returns late', async () => {
  let finishOld!: (v: TimelineDay[]) => void;
  invoke.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishOld = resolve;
      }),
  );
  invoke.mockResolvedValueOnce([days[1]]);
  const { wrapper } = await setup();
  await wrapper.get('select').setValue('trip');
  await flushPromises();
  finishOld([days[0]!]);
  await flushPromises();
  expect(wrapper.findAll('.timeline-day')).toHaveLength(1);
  expect(wrapper.findAll('.timeline-day')[0]!.text()).toContain('未知日期');
  expect(wrapper.findAll('.timeline-summary strong').map((n) => n.text())).toEqual(['0', '1', '0', '1']);
});
it('shows a persistent read failure and loads a fresh result when retried', async () => {
  invoke.mockRejectedValueOnce(new Error('读取失败'));
  const { wrapper } = await setup();
  expect(wrapper.get('[role="alert"]').text()).toContain('读取失败');
  expect(wrapper.find('.timeline-summary').exists()).toBe(false);
  invoke.mockResolvedValueOnce(days);
  await wrapper.get('button').trigger('click');
  await flushPromises();
  expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  expect(wrapper.findAll('.timeline-day')).toHaveLength(2);
});
