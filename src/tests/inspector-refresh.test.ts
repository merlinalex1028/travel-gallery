import { expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import Inspector from '../components/Inspector.vue';
import { useLibrary } from '../stores/library';
import type { AssetDetail, Commands } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((finish) => { resolve = finish; });
  return { promise, resolve };
}

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

it.each([false, true])('keeps the newly selected photo when an earlier time save finishes (details loaded: %s)', async (loaded) => {
  setActivePinia(createPinia());
  const lib = useLibrary();
  const first = { ...(structuredClone(golden.assetDetail) as AssetDetail), id: 'first', filename: 'first.jpg' };
  const second = { ...first, id: 'second', filename: 'second.jpg' };
  lib.detail = first;
  lib.focusId = first.id;
  const revision = (first.captureTimeCorrection?.revision ?? 0) + 1;
  const save = deferred<Commands['save_capture_time']['result']>();
  const nextDetail = deferred<AssetDetail>();
  invoke.mockReset();
  invoke.mockImplementation((name: string, request: { assetId: string }) => {
    if (name === 'get_media_links') return Promise.resolve([]);
    if (name === 'save_capture_time') return save.promise;
    if (name === 'get_asset') return request.assetId === second.id ? nextDetail.promise : Promise.resolve(first);
    if (name === 'query_assets') return Promise.resolve({ items: [], total: 0, nextCursor: null, queryFingerprint: '' });
    throw new Error(`unexpected ${name}`);
  });
  const wrapper = mount(Inspector, {
    global: {
      stubs: { MediaThumbnail: true, ReviewControls: true, RouterLink: true, LinkedPreviewDialog: true },
    },
  });
  const beginEdit = () => wrapper.findAll('button').find((b) => b.text() === '修正拍摄时间')!.trigger('click');
  try {
    await beginEdit();
    await wrapper.get('input[type="datetime-local"]').setValue('2026-06-15T11:00:00');
    await wrapper.get('form').trigger('submit');
    lib.select(second);
    if (loaded) {
      nextDetail.resolve(second);
      await flushPromises();
      await beginEdit();
      await wrapper.get('input[type="datetime-local"]').setValue('2026-06-16T12:00:00');
    }
    save.resolve({ updated: true, revision });
    await flushPromises();
    nextDetail.resolve(second);
    await flushPromises();
    expect(lib.focusId).toBe(second.id);
    expect(wrapper.get('.inspector-filename').text()).toBe(second.filename);
    if (loaded) {
      expect(wrapper.get<HTMLInputElement>('input[type="datetime-local"]').element.value).toBe('2026-06-16T12:00');
    }
    expect(invoke).toHaveBeenCalledWith('save_capture_time', expect.objectContaining({ assetId: first.id }));
    expect(invoke).toHaveBeenCalledWith('query_assets', expect.anything());
  } finally {
    wrapper.unmount();
  }
});
