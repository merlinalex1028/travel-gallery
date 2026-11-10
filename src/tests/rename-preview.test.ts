import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import RenamePreviewDialog from '../components/RenamePreviewDialog.vue';
import type { RenamePreview } from '../services/contracts';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));
function report(): RenamePreview {
  return {
    job: {
      id: 'preview',
      type: 'REPAIR',
      operation: 'renamePreview',
      status: 'completed',
      progressDone: 1,
      progressTotal: 1,
      failureCount: 0,
      errorSummary: null,
      createdAt: 0,
      updatedAt: 0,
    },
    rule: { pattern: '{date}_{seq}', start: 1, padding: 4 },
    target: { kind: 'snapshot', snapshotId: 'frozen' },
    summary: { total: 1, ready: 1, unchanged: 0, conflict: 0, invalid: 0, unavailable: 0 },
    items: [
      {
        assetId: 'original',
        sourceId: 'source',
        sourceName: '来源',
        sourceRelativePath: '原图.jpg',
        originalFilename: '原图.jpg',
        proposedFilename: '20260615_0001.jpg',
        proposedRelativePath: '20260615_0001.jpg',
        status: 'ready',
        reason: '',
        dateUnknown: false,
      },
    ],
    nextOffset: null,
    expiresAt: 9999999999999,
  };
}
const options = {
  global: {
    stubs: {
      ElDialog: {
        props: ['modelValue'],
        template: '<div v-if="modelValue"><slot/><slot name="footer"/></div>',
      },
      ElProgress: true,
    },
  },
};
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
});
it('freezes the opening selection and transfers the server snapshot to copy export', async () => {
  const app = useApp(),
    lib = useLibrary();
  lib.selected = new Set(['original']);
  invoke.mockImplementation(async (name: string) => {
    if (name === 'preview_rename') return { jobId: 'preview' };
    if (name === 'get_rename_preview') return report();
    throw new Error(name);
  });
  const wrapper = mount(RenamePreviewDialog, options);
  app.renameOpen = true;
  await flushPromises();
  lib.selected = new Set(['later']);
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '开始预演')!
    .trigger('click');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith('preview_rename', {
    target: { kind: 'explicit', assetIds: ['original'] },
    rule: { pattern: '{date}_{seq}', start: 1, padding: 4 },
  });
  expect(wrapper.text()).toContain('20260615_0001.jpg');
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '用于复制导出')!
    .trigger('click');
  expect(app.exportDraft).toEqual({
    target: { kind: 'snapshot', snapshotId: 'frozen' },
    count: 1,
    naming: report().rule,
  });
  expect(app.exportOpen).toBe(true);
  expect(app.renameOpen).toBe(false);
  wrapper.unmount();
});
it('does not install a late report into a newly opened preview', async () => {
  const app = useApp(),
    lib = useLibrary();
  lib.selected = new Set(['a']);
  app.renameJobId = 'old';
  let finish!: (value: RenamePreview) => void;
  invoke.mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const wrapper = mount(RenamePreviewDialog, options);
  app.renameOpen = true;
  await flushPromises();
  app.renameOpen = false;
  await flushPromises();
  app.renameJobId = '';
  lib.selected = new Set(['b']);
  app.renameOpen = true;
  await flushPromises();
  finish(report());
  await flushPromises();
  expect(wrapper.text()).not.toContain('20260615_0001.jpg');
  expect(wrapper.text()).toContain('开始预演');
  expect(app.exportDraft).toBeNull();
  wrapper.unmount();
});
