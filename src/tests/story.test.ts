import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { createMemoryHistory, createRouter } from 'vue-router';
import StoryPage from '../pages/StoryPage.vue';
import { useApp } from '../stores/app';
import type { StoryExportPreview, TripStory } from '../services/contracts';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));
const mounted: { unmount(): void }[] = [];
afterEach(() => mounted.splice(0).forEach((w) => w.unmount()));
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
  vi.spyOn(useApp(), 'refresh').mockResolvedValue();
});
const story: TripStory = {
  trip: {
    id: 'trip',
    name: '旅途 A',
    description: '自己的回忆',
    coverAssetId: 'a',
    startAt: null,
    endAt: null,
    count: 3,
  },
  items: [
    {
      assetId: 'a',
      filename: 'a.jpg',
      date: '2026-06-15',
      unknownTimezone: true,
      note: '私人文字',
      mediaKind: 'image',
      fileState: 'available',
    },
    {
      assetId: 'b',
      filename: 'b.mov',
      date: null,
      unknownTimezone: false,
      note: '',
      mediaKind: 'video',
      fileState: 'available',
    },
  ],
  total: 3,
  nextOffset: 2,
  epoch: 7,
};
const plan: StoryExportPreview = {
  planToken: 'frozen-plan',
  itemCount: 3,
  offlineCount: 0,
  folder: 'travel-story-test',
  expiresAt: 9999999999999,
  includeNotes: false,
};
async function setup() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/trips/:id/story', component: { render: () => null } },
      { path: '/jobs', component: { render: () => null } },
      { path: '/trips/:id', component: { render: () => null } },
    ],
  });
  await router.push('/trips/trip/story');
  const wrapper = mount(StoryPage, {
    global: {
      plugins: [router],
      stubs: {
        Icon: true,
        MediaThumbnail: true,
        ElDialog: {
          props: ['modelValue'],
          emits: ['update:modelValue'],
          template:
            '<div v-if="modelValue"><button class="close-dialog" @click="$emit(\'update:modelValue\',false)">关闭</button><slot/></div>',
        },
      },
    },
  });
  mounted.push(wrapper);
  await flushPromises();
  return { wrapper, router };
}
it('shows trusted chapters and video posters, and pages using the same catalog epoch', async () => {
  invoke.mockResolvedValueOnce(story).mockResolvedValueOnce({
    ...story,
    items: [{ ...story.items[1]!, assetId: 'c', filename: 'c.mov' }],
    nextOffset: null,
  });
  const { wrapper } = await setup();
  expect(wrapper.findAll('.story-chapter h2').map((n) => n.text())).toEqual(['2026-06-15', '未知拍摄日期']);
  expect(wrapper.text()).toContain('视频静帧');
  expect(wrapper.text()).toContain('时区未知');
  await wrapper.get('.story-more button').trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenLastCalledWith('get_trip_story', {
    tripId: 'trip',
    range: 'all',
    offset: 2,
    pageSize: 100,
    epoch: 7,
  });
  expect(wrapper.findAll('.story-frame')).toHaveLength(3);
  expect(wrapper.find('.story-more').exists()).toBe(false);
  invoke.mockResolvedValueOnce({ ...story, items: [], total: 0, nextOffset: null });
  await wrapper.get('select').setValue('keep');
  await flushPromises();
  expect(invoke).toHaveBeenLastCalledWith('get_trip_story', {
    tripId: 'trip',
    range: 'keep',
    offset: 0,
    pageSize: 100,
    epoch: null,
  });
  expect(wrapper.text()).toContain('这里还没有故事内容');
});
it('ignores an old trip response and refreshes after a stale page without mixing epochs', async () => {
  let finish!: (v: TripStory) => void;
  invoke.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const { wrapper, router } = await setup();
  invoke.mockResolvedValueOnce({ ...story, trip: { ...story.trip, id: 'new', name: '新旅途' } });
  await router.push('/trips/new/story');
  await flushPromises();
  finish({ ...story, trip: { ...story.trip, name: '不该显示' } });
  await flushPromises();
  expect(wrapper.text()).toContain('新旅途');
  expect(wrapper.text()).not.toContain('不该显示');
  invoke.mockRejectedValueOnce(new Error('旅行内容已变化'));
  await wrapper.get('.story-more button').trigger('click');
  await flushPromises();
  expect(wrapper.get('[role="alert"]').text()).toContain('旅行内容已变化');
  expect(wrapper.findAll('.story-frame')).toHaveLength(2);
  invoke.mockResolvedValueOnce({ ...story, items: [story.items[0]!], nextOffset: null, total: 1, epoch: 8 });
  await wrapper.get('[role="alert"] button').trigger('click');
  await flushPromises();
  expect(wrapper.findAll('.story-frame')).toHaveLength(1);
});
it('defaults notes off, discards a late export preview after edits, and reuses an idempotency key on retry', async () => {
  invoke.mockResolvedValueOnce(story);
  const { wrapper } = await setup();
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '导出离线故事')!
    .trigger('click');
  expect(wrapper.get<HTMLInputElement>('input[type="checkbox"]').element.checked).toBe(false);
  invoke.mockResolvedValueOnce({ path: '/owned-export' });
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '选择文件夹')!
    .trigger('click');
  await flushPromises();
  let finish!: (v: StoryExportPreview) => void;
  invoke.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await wrapper.get('form').trigger('submit');
  expect(invoke).toHaveBeenLastCalledWith(
    'preview_story_export',
    expect.objectContaining({
      tripId: 'trip',
      range: 'all',
      includeNotes: false,
      destinationPath: '/owned-export',
    }),
  );
  await wrapper.get('.close-dialog').trigger('click');
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '导出离线故事')!
    .trigger('click');
  await wrapper.get('input[maxlength="100"]').setValue('修改后的故事');
  finish(plan);
  await flushPromises();
  expect(wrapper.find('.story-export-preview').exists()).toBe(false);
  invoke.mockResolvedValueOnce({ path: '/owned-export' });
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '选择文件夹')!
    .trigger('click');
  await flushPromises();
  invoke.mockResolvedValueOnce(plan);
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(wrapper.find('.story-export-preview').exists()).toBe(true);
  invoke.mockRejectedValueOnce(new Error('暂时失败'));
  await wrapper
    .findAll('button')
    .find((b) => b.text().startsWith('开始导出'))!
    .trigger('click');
  await flushPromises();
  const first = invoke.mock.calls.at(-1);
  expect(wrapper.text()).toContain('暂时失败');
  invoke.mockResolvedValueOnce({ jobId: 'story-job' });
  await wrapper
    .findAll('button')
    .find((b) => b.text().startsWith('开始导出'))!
    .trigger('click');
  await flushPromises();
  expect(invoke.mock.calls.at(-1)).toEqual(first);
  expect(wrapper.text()).toContain('故事任务已加入后台');
});
it('invalidates a plan when notes change and restores privacy defaults when reopened', async () => {
  invoke.mockResolvedValueOnce(story);
  const { wrapper } = await setup();
  const open = () =>
    wrapper
      .findAll('button')
      .find((b) => b.text() === '导出离线故事')!
      .trigger('click');
  await open();
  invoke.mockResolvedValueOnce({ path: '/owned-export' });
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '选择文件夹')!
    .trigger('click');
  await flushPromises();
  invoke.mockResolvedValueOnce(plan);
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  await wrapper.get('input[type="checkbox"]').setValue(true);
  expect(wrapper.find('.story-export-preview').exists()).toBe(false);
  await wrapper.get('.close-dialog').trigger('click');
  await open();
  expect(wrapper.get<HTMLInputElement>('input[type="checkbox"]').element.checked).toBe(false);
  expect(wrapper.get<HTMLInputElement>('[aria-label="故事导出目录"]').element.value).toBe('');
});
