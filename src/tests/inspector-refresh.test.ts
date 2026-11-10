import { expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import Inspector from '../components/Inspector.vue';
import { useLibrary } from '../stores/library';
import type { AssetDetail } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));
it('refreshes metadata without discarding drafts or silently rebasing a capture-time edit', async () => {
  setActivePinia(createPinia());
  const lib = useLibrary();
  lib.detail = structuredClone(golden.assetDetail) as AssetDetail;
  invoke.mockImplementation((name: string) => {
    if (name === 'get_media_links') return Promise.resolve([]);
    if (name === 'save_capture_time') throw new Error('拍摄时间已改变，请重新编辑');
    throw new Error(`unexpected ${name}`);
  });
  const wrapper = mount(Inspector, {
    global: {
      stubs: { MediaThumbnail: true, ReviewControls: true, RouterLink: true, LinkedPreviewDialog: true },
    },
  });
  await flushPromises();
  const note = wrapper.get<HTMLTextAreaElement>('[aria-label="照片备注"]');
  note.element.value = '尚未提交的备注';
  await note.trigger('input');
  await wrapper
    .findAll('button')
    .find((b) => b.text() === '修正拍摄时间')!
    .trigger('click');
  await wrapper.get('input[type="datetime-local"]').setValue('2026-06-15T11:00:00');
  const revision = lib.detail.captureTimeCorrection!.revision;
  lib.detail = {
    ...lib.detail,
    width: 7000,
    captureTimeCorrection: { ...lib.detail.captureTimeCorrection!, revision: revision + 1 },
  };
  await flushPromises();
  expect(wrapper.text()).toContain('7000 ×');
  expect(note.element.value).toBe('尚未提交的备注');
  expect(wrapper.get<HTMLInputElement>('input[type="datetime-local"]').element.value).toBe(
    '2026-06-15T11:00',
  );
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith(
    'save_capture_time',
    expect.objectContaining({ expectedRevision: revision, captureLocal: '2026-06-15 11:00:00' }),
  );
  expect(wrapper.find('input[type="datetime-local"]').exists()).toBe(true);
  expect(note.element.value).toBe('尚未提交的备注');
  wrapper.unmount();
});
