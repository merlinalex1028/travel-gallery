import { expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { nextTick } from 'vue';
import ExportWizard from '../components/ExportWizard.vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import type { ExportPreview } from '../services/contracts';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));
it('opts out of XMP on each opening and ignores a preview from a closed dialog', async () => {
  setActivePinia(createPinia());
  const app = useApp(),
    lib = useLibrary();
  lib.selected = new Set(['a']);
  let resolveOld!: (value: ExportPreview) => void;
  const result: ExportPreview = {
    planToken: 'old-plan',
    expiresAt: 9999999999999,
    fileCount: 1,
    xmpCount: 1,
    totalBytes: 5000,
    conflictCount: 0,
    offlineCount: 0,
    duplicateDestinationCount: 0,
    warnings: [],
  };
  invoke.mockImplementation((name: string) => {
    if (name === 'pick_directory') return Promise.resolve({ path: '/export-fixture' });
    if (name === 'preview_export')
      return new Promise((resolve) => {
        resolveOld = resolve;
      });
    throw new Error(name);
  });
  const wrapper = mount(ExportWizard, {
    global: {
      stubs: {
        ElDialog: {
          props: ['modelValue'],
          template: '<div v-if="modelValue"><slot/><slot name="footer"/></div>',
        },
        ElProgress: true,
        RouterLink: true,
      },
    },
  });
  app.exportOpen = true;
  await nextTick();
  const xmp = () => wrapper.get<HTMLInputElement>('input[type="checkbox"]').element;
  expect(wrapper.findAll<HTMLInputElement>('input[type="checkbox"]')[1]!.element.checked).toBe(false);
  await wrapper.get('.folder-picker').trigger('click');
  await flushPromises();
  await wrapper.findAll('input[type="checkbox"]')[1]!.setValue(true);
  lib.selected = new Set(['b']);
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '预演导出')!
    .trigger('click');
  expect(invoke).toHaveBeenCalledWith(
    'preview_export',
    expect.objectContaining({ target: { kind: 'explicit', assetIds: ['a'] }, includeXmp: true }),
  );
  expect(xmp().disabled).toBe(true);
  app.exportOpen = false;
  await nextTick();
  app.exportOpen = true;
  await nextTick();
  expect(xmp().disabled).toBe(false);
  expect(wrapper.text()).not.toContain('确认复制');
  expect(wrapper.findAll<HTMLInputElement>('input[type="checkbox"]')[1]!.element.checked).toBe(false);
  let resolveNew!: (value: ExportPreview) => void;
  invoke.mockImplementation((name: string) =>
    name === 'pick_directory'
      ? Promise.resolve({ path: '/new-export' })
      : new Promise((resolve) => {
          resolveNew = resolve;
        }),
  );
  await wrapper.get('.folder-picker').trigger('click');
  await flushPromises();
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '预演导出')!
    .trigger('click');
  await flushPromises();
  resolveOld(result);
  await flushPromises();
  expect(xmp().disabled).toBe(true);
  expect(wrapper.text()).not.toContain('确认复制');
  resolveNew({ ...result, planToken: 'new-plan', xmpCount: 0 });
  await flushPromises();
  expect(wrapper.text()).toContain('确认复制 1 个媒体');
  expect(wrapper.text()).not.toContain('+ 1 个 XMP');
  expect(invoke).toHaveBeenLastCalledWith(
    'preview_export',
    expect.objectContaining({ target: { kind: 'explicit', assetIds: ['b'] }, includeXmp: false }),
  );
  wrapper.unmount();
});

it('applies a saved preset explicitly and saves rules without destination authority or selection', async () => {
  setActivePinia(createPinia());
  const app = useApp(),
    lib = useLibrary();
  lib.selected = new Set(['original']);
  const preset = {
    id: 'preset',
    name: '旅行精选',
    revision: 3,
    options: {
      layout: 'captureMonth',
      conflictPolicy: 'skip',
      includeLinked: false,
      includeXmp: true,
      naming: { pattern: '旅行_{seq}', start: 7, padding: 4 },
    },
  };
  invoke.mockImplementation(async (name: string, r: any) => {
    if (name === 'list_export_presets') return [preset];
    if (name === 'pick_directory') return { path: '/authorized-once' };
    if (name === 'save_export_preset') return { ...preset, ...r, revision: 4 };
    if (name === 'preview_export')
      return {
        planToken: 'planned',
        fileCount: 1,
        xmpCount: 1,
        totalBytes: 10,
        conflictCount: 0,
        offlineCount: 0,
        warnings: [],
      };
    throw new Error(name);
  });
  const wrapper = mount(ExportWizard, {
    global: {
      stubs: {
        ElDialog: {
          props: ['modelValue'],
          template: '<div v-if="modelValue"><slot/><slot name="footer"/></div>',
        },
        RouterLink: true,
        ElProgress: true,
      },
    },
  });
  app.exportOpen = true;
  await flushPromises();
  const checks = () => wrapper.findAll<HTMLInputElement>('input[type="checkbox"]');
  expect(checks()[1]!.element.checked).toBe(false);
  await wrapper.get('select[aria-label="导出预设"]').setValue('preset');
  expect(checks()[1]!.element.checked).toBe(true);
  expect(wrapper.get<HTMLInputElement>('[aria-label="文件名模板"]').element.value).toBe('旅行_{seq}');
  await wrapper.get('.folder-picker').trigger('click');
  await flushPromises();
  await wrapper.get('[aria-label="预设名称"]').setValue('更新的精选');
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '更新预设')!
    .trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith('save_export_preset', {
    id: 'preset',
    expectedRevision: 3,
    name: '更新的精选',
    options: preset.options,
  });
  lib.selected = new Set(['later']);
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '预演导出')!
    .trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenLastCalledWith(
    'preview_export',
    expect.objectContaining({
      destinationPath: '/authorized-once',
      target: { kind: 'explicit', assetIds: ['original'] },
      naming: preset.options.naming,
    }),
  );
  await wrapper.get('[aria-label="文件名模板"]').setValue('另一个_{seq}');
  expect(wrapper.text()).not.toContain('确认复制');
  wrapper.unmount();
});
