import { setSelect } from './helpers/select';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';
import QualityPage from '../pages/QualityPage.vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import type { QualityStatus, QualityResults, AssetSummary } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message, isDemo: false }));
vi.mock('element-plus', () => ({
  ElMessage: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
  ElMessageBox: { confirm: vi.fn().mockResolvedValue(true) },
}));
const mounted: { unmount(): void }[] = [];
afterEach(() => mounted.splice(0).forEach((w) => w.unmount()));
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
  vi.spyOn(useApp(), 'refresh').mockResolvedValue();
  useApp().trips = [
    {
      id: 'trip',
      name: '验收旅行',
      description: null,
      startAt: null,
      endAt: null,
      coverAssetId: null,
      count: 1,
    },
  ];
});
const status: QualityStatus = {
  eyesEnabled: false,
  detailEnabled: false,
  modelAvailable: false,
  modelFolderPresent: false,
  profileId: 'face-v1',
  profileName: 'Face Landmarker',
  modelBytes: 10005871,
  installedAt: null,
  runtime: 'idle',
  inferenceThreads: 1,
  idleReleaseSeconds: 60,
  total: 3,
  eyesCount: 0,
  detailCount: 0,
  eyeVersion: 'eye-test-v1',
  detailVersion: 'detail-test-v1',
};
const empty: QualityResults = { items: [], offset: 0, total: 0, pageSize: 60 };
const one: QualityResults = {
  ...empty,
  total: 1,
  items: [
    {
      asset: { ...golden.assetSummary, id: 'face', filename: 'sample.jpg' } as AssetSummary,
      analysis: {
        assetId: 'face',
        staleCount: 0,
        observations: [
          {
            kind: 'eyes',
            version: 'eye-test-v1',
            inputKey: 'a'.repeat(64),
            createdAt: 1,
            dismissed: false,
            value: {
              width: 800,
              height: 600,
              faceLimitReached: false,
              faces: [
                {
                  bounds: [0.1, 0.1, 0.4, 0.4],
                  blinkLeft: 0.7,
                  blinkRight: 0.8,
                  eyeRatioLeft: 0.1,
                  eyeRatioRight: 0.1,
                  status: 'possibleClosed',
                  reason: 'bothEyes',
                },
              ],
            },
          },
        ],
      },
    },
  ],
};
function mockBase(s = status, r = empty) {
  invoke.mockImplementation((name: string) => {
    if (name === 'get_quality_status') return Promise.resolve(s);
    if (name === 'get_quality_results') return Promise.resolve(r);
    throw new Error(name);
  });
}
async function setup() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/quality', '/compare', '/viewer/:assetId', '/jobs'].map((path) => ({
      path,
      component: { render: () => null },
    })),
  });
  await router.push('/quality');
  const wrapper = mount(QualityPage, {
    global: {
      plugins: [router],
      stubs: {
        Icon: true,
        QualityPreview: true,
        MediaThumbnail: true,
        EmptyState: { props: ['title', 'description'], template: '<div>{{title}} {{description}}</div>' },
        ElDialog: { props: ['modelValue'], template: '<div v-if="modelValue"><slot/></div>' },
      },
    },
  });
  mounted.push(wrapper);
  await flushPromises();
  return { wrapper, router, lib: useLibrary() };
}
function button(w: Awaited<ReturnType<typeof setup>>['wrapper'], text: string) {
  return w.findAll('button').find((b) => b.text().includes(text))!;
}
it('loads only records by default and never opts in or starts inference automatically', async () => {
  mockBase();
  const { wrapper } = await setup();
  expect(wrapper.text()).toContain('默认关闭');
  expect(button(wrapper, '启用闭眼提示').attributes('disabled')).toBeDefined();
  expect(button(wrapper, '检查当前范围').attributes('disabled')).toBeDefined();
  expect(
    invoke.mock.calls.every(([name]) => ['get_quality_status', 'get_quality_results'].includes(name)),
  ).toBe(true);
});
it('does not display late results from the previous travel scope', async () => {
  let finish!: (v: QualityResults) => void;
  invoke.mockImplementation((name: string, r: { filter: { collectionId?: string } }) =>
    name === 'get_quality_status'
      ? Promise.resolve(status)
      : r.filter.collectionId
        ? Promise.resolve(empty)
        : new Promise((resolve) => {
            finish = resolve;
          }),
  );
  const { wrapper } = await setup();
  await setSelect(wrapper, '检查旅行范围', 'trip');
  await flushPromises();
  finish(one);
  await flushPromises();
  expect(wrapper.findAll('.quality-card')).toHaveLength(0);
  expect(wrapper.text()).not.toContain('sample.jpg');
});
it('dismisses only the returned observation version and does not send a human-review mutation', async () => {
  mockBase({ ...status, eyesEnabled: true, modelAvailable: true }, one);
  const { wrapper } = await setup();
  invoke.mockImplementation((name: string) => {
    if (name === 'set_quality_dismissed') return Promise.resolve({ updated: true });
    if (name === 'get_quality_status') return Promise.resolve(status);
    if (name === 'get_quality_results') return Promise.resolve(empty);
    throw new Error(name);
  });
  await button(wrapper, '忽略此记录').trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith(
    'set_quality_dismissed',
    expect.objectContaining({ assetId: 'face', kind: 'eyes', inputKey: 'a'.repeat(64), dismissed: true }),
  );
  expect(invoke.mock.calls.some(([n]) => n === 'apply_review_batch')).toBe(false);
  expect(wrapper.findAll('.quality-card')).toHaveLength(0);
});
it('does not import after unmounting during the native picker', async () => {
  mockBase();
  const { wrapper } = await setup();
  let finish!: (v: { path: string }) => void;
  invoke.mockImplementation((name: string) =>
    name === 'pick_directory'
      ? new Promise((resolve) => {
          finish = resolve;
        })
      : Promise.resolve({}),
  );
  await button(wrapper, '导入离线模型包').trigger('click');
  wrapper.unmount();
  finish({ path: '/owned/face-package' });
  await flushPromises();
  expect(invoke.mock.calls.some(([n]) => n === 'import_eyes_model')).toBe(false);
});
it('freezes the enabled analysis types and scope when submitting a background task', async () => {
  mockBase({ ...status, detailEnabled: true });
  const { wrapper } = await setup();
  await setSelect(wrapper, '检查旅行范围', 'trip');
  await flushPromises();
  invoke.mockImplementation((name: string) =>
    name === 'start_quality_analysis'
      ? Promise.resolve({ jobId: 'quality-job' })
      : name === 'get_quality_status'
        ? Promise.resolve(status)
        : Promise.resolve(empty),
  );
  await button(wrapper, '检查当前范围').trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith(
    'start_quality_analysis',
    expect.objectContaining({
      filter: { combineLinked: false, collectionId: 'trip' },
      useEyes: false,
      useDetail: true,
    }),
  );
});

it('reads the face profile and labels project hint rules without attributing them to the model author', async () => {
  mockBase();
  const { wrapper } = await setup();
  invoke.mockImplementation((name: string) =>
    name === 'get_eyes_model_info'
      ? Promise.resolve({
          profile: { repository: 'face-model', revision: 'v1', license: 'Apache-2.0' },
          card: 'Project hint rules',
          licenseText: 'Apache License',
        })
      : name === 'get_quality_status'
        ? Promise.resolve(status)
        : Promise.resolve(empty),
  );
  await button(wrapper, '模型说明与许可').trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith('get_eyes_model_info', undefined);
  expect(wrapper.text()).toContain('模型与提示规则');
  expect(wrapper.text()).not.toContain('作者模型说明');
});
