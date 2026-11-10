import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, flushPromises } from '@vue/test-utils';
import { createRouter, createMemoryHistory } from 'vue-router';
import LocalAiPage from '../pages/LocalAiPage.vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import type { AiStatus, SemanticResults } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({
  api: invoke,
  errorMessage: (error: Error) => error.message,
  isDemo: false,
}));
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
  vi.spyOn(useLibrary(), 'search').mockResolvedValue();
  useApp().trips = [
    {
      id: 'trip',
      name: '公开验收旅行',
      description: null,
      startAt: null,
      endAt: null,
      coverAssetId: null,
      count: 3,
    },
  ];
});
const enabled: AiStatus = {
  enabled: true,
  modelAvailable: true,
  modelFolderPresent: true,
  modelVersion: 'siglip2-test',
  profileId: 'siglip2-test',
  profileName: 'Google SigLIP 2 Base',
  modelBytes: 1535226485,
  installedAt: 1,
  runtime: 'loaded',
  inferenceThreads: 2,
  idleReleaseSeconds: 60,
  total: 3,
  indexedCount: 3,
};
const emptyGroups = { jobId: null, groups: [], summary: null, createdAt: null, filter: {} };
const result = {
  items: [{ asset: { ...golden.assetSummary, id: 'picture' }, score: 0.2 }],
  total: 3,
  indexedCount: 3,
  invalidCount: 0,
  modelVersion: 'siglip2-test',
  limit: 60,
} as SemanticResults;
function mockBase(status: AiStatus = enabled) {
  invoke.mockImplementation((name: string) => {
    if (name === 'get_ai_status') return Promise.resolve(status);
    if (name === 'get_ai_clusters') return Promise.resolve(emptyGroups);
    throw new Error(name);
  });
}
async function setup() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/ai', '/filter', '/jobs'].map((path) => ({ path, component: { render: () => null } })),
  });
  await router.push('/ai');
  const wrapper = mount(LocalAiPage, {
    global: {
      plugins: [router],
      stubs: {
        Icon: true,
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
function button(wrapper: Awaited<ReturnType<typeof setup>>['wrapper'], text: string) {
  return wrapper.findAll('button').find((button) => button.text().includes(text))!;
}
it('defaults to disabled and never starts a model, import or semantic request automatically', async () => {
  mockBase({
    ...enabled,
    enabled: false,
    modelAvailable: false,
    modelFolderPresent: false,
    indexedCount: 0,
    runtime: 'idle',
  });
  const { wrapper } = await setup();
  expect(wrapper.text()).toContain('默认关闭');
  expect(button(wrapper, '启用本地 AI').attributes('disabled')).toBeDefined();
  expect(wrapper.find('form').exists()).toBe(false);
  expect(invoke.mock.calls.every(([name]) => ['get_ai_status', 'get_ai_clusters'].includes(name))).toBe(true);
});
it('ignores semantic results from an old scope and leaves the library untouched', async () => {
  mockBase();
  const { wrapper, lib } = await setup();
  lib.setFilter({ favorite: true });
  let finish!: (v: SemanticResults) => void;
  invoke.mockImplementation((name: string) =>
    name === 'semantic_search'
      ? new Promise((resolve) => {
          finish = resolve;
        })
      : name === 'get_ai_status'
        ? Promise.resolve(enabled)
        : Promise.resolve(emptyGroups),
  );
  await wrapper.get('[aria-label="语义搜索描述"]').setValue('一座教堂');
  await wrapper.get('form').trigger('submit');
  expect(invoke).toHaveBeenCalledWith('semantic_search', {
    filter: { combineLinked: false },
    text: '一座教堂',
    limit: 60,
  });
  await wrapper.get('[aria-label="AI 旅行范围"]').setValue('trip');
  await flushPromises();
  finish(result);
  await flushPromises();
  expect(wrapper.find('.ai-results').exists()).toBe(false);
  expect(lib.filter).toEqual({ favorite: true });
  expect(invoke).toHaveBeenCalledWith('get_ai_status', {
    filter: { combineLinked: false, collectionId: 'trip' },
  });
});
it('disables the model during a pending search and discards its late result', async () => {
  let current = enabled;
  let finish!: (v: SemanticResults) => void;
  invoke.mockImplementation((name: string) => {
    if (name === 'get_ai_status') return Promise.resolve(current);
    if (name === 'get_ai_clusters') return Promise.resolve(emptyGroups);
    if (name === 'semantic_search')
      return new Promise((resolve) => {
        finish = resolve;
      });
    if (name === 'set_ai_enabled') {
      current = { ...enabled, enabled: false, runtime: 'idle' };
      return Promise.resolve({ updated: true });
    }
    throw new Error(name);
  });
  const { wrapper } = await setup();
  await wrapper.get('[aria-label="语义搜索描述"]').setValue('城市街道');
  await wrapper.get('form').trigger('submit');
  await button(wrapper, '关闭并释放模型').trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith('set_ai_enabled', { enabled: false });
  finish(result);
  await flushPromises();
  expect(wrapper.find('.ai-results').exists()).toBe(false);
  expect(wrapper.find('form').exists()).toBe(false);
});
it('opens exactly the returned results and rejects a changed snapshot count', async () => {
  mockBase();
  const { wrapper, router, lib } = await setup();
  invoke.mockImplementation((name: string) => {
    if (name === 'semantic_search') return Promise.resolve(result);
    if (name === 'create_selection_snapshot') return Promise.resolve({ id: 'result-snapshot', count: 0 });
    if (name === 'get_ai_status') return Promise.resolve(enabled);
    return Promise.resolve(emptyGroups);
  });
  await wrapper.get('[aria-label="语义搜索描述"]').setValue('教堂');
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(wrapper.get('.ai-result-card').text()).toContain('相近程度 0.200');
  await button(wrapper, '在图库查看本次').trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith(
    'create_selection_snapshot',
    expect.objectContaining({ target: { kind: 'explicit', assetIds: ['picture'] } }),
  );
  expect(wrapper.get('[role="alert"]').text()).toContain('部分结果已移除');
  expect(router.currentRoute.value.path).toBe('/ai');
  expect(lib.filter).toEqual({});
  invoke.mockImplementation((name: string) =>
    name === 'create_selection_snapshot'
      ? Promise.resolve({ id: 'result-snapshot', count: 1 })
      : Promise.resolve({}),
  );
  await button(wrapper, '在图库查看本次').trigger('click');
  await flushPromises();
  expect(router.currentRoute.value.fullPath).toBe('/filter?showResults=1');
  expect(lib.filter).toEqual({ selectionSnapshotId: 'result-snapshot', combineLinked: false });
});
it('does not begin a model import after its page was closed while the picker was open', async () => {
  mockBase({ ...enabled, enabled: false, modelAvailable: false, modelFolderPresent: false, indexedCount: 0 });
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
  finish({ path: '/owned/model-package' });
  await flushPromises();
  expect(invoke.mock.calls.some(([name]) => name === 'import_ai_model')).toBe(false);
});
