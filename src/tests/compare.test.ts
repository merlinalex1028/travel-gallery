import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { flushPromises, mount } from '@vue/test-utils';
import ComparePage from '../pages/ComparePage.vue';
import { useLibrary } from '../stores/library';
import type { AssetDetail } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';
const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: (e: Error) => e.message }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));
let records: AssetDetail[];
const mounted: { unmount(): void }[] = [];
afterEach(() => mounted.splice(0).forEach((wrapper) => wrapper.unmount()));
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
  records = ['a', 'b', 'c', 'd'].map((id) => ({
    ...(structuredClone(golden.assetDetail) as AssetDetail),
    id,
    filename: `${id}.jpg`,
  }));
  const batches = new Map<string, { assetId: string; before: AssetDetail['review'] }>();
  let revision = 0;
  invoke.mockImplementation(async (command: string, r: any) => {
    if (command === 'get_asset') return structuredClone(records.find((a) => a.id === r.assetId));
    if (command === 'query_assets')
      return { items: structuredClone(records), total: 4, nextCursor: null, queryFingerprint: 'all' };
    if (command === 'apply_review_batch') {
      const asset = records.find((a) => a.id === r.target.assetIds[0])!;
      const id = `batch-${++revision}`;
      batches.set(id, { assetId: asset.id, before: structuredClone(asset.review) });
      Object.assign(asset.review, r.patch, { revision: asset.review.revision + 1 });
      return { changeBatchId: id, affectedCount: 1, unchangedCount: 0, conflicts: [] };
    }
    if (command === 'undo_review_batch') {
      const batch = batches.get(r.changeBatchId)!;
      const asset = records.find((a) => a.id === batch.assetId)!;
      asset.review = { ...batch.before, revision: asset.review.revision + 1 };
      return { changeBatchId: r.changeBatchId, restoredCount: 1, skippedCount: 0 };
    }
    throw new Error(command);
  });
});
async function setup() {
  const lib = useLibrary();
  lib.selected = new Set(records.map((a) => a.id));
  const router = createRouter({
    history: createMemoryHistory(),
    routes: ['/compare', '/library'].map((path) => ({ path, component: { render: () => null } })),
  });
  await router.push('/compare');
  await router.isReady();
  const wrapper = mount(ComparePage, {
    global: {
      plugins: [router],
      stubs: { MediaStage: { props: ['asset'], template: '<div>{{asset.filename}}</div>' } },
    },
  });
  mounted.push(wrapper);
  await flushPromises();
  return { wrapper, lib };
}
it('marks only the chosen fourth pane and refreshes every pane after the undo button', async () => {
  const { wrapper, lib } = await setup();
  expect(wrapper.findAll('article')).toHaveLength(4);
  await wrapper.findAll('article')[3]!.get('button').trigger('click');
  await flushPromises();
  expect(records.map((a) => a.review.pickState)).toEqual(['unreviewed', 'unreviewed', 'unreviewed', 'keep']);
  expect(wrapper.findAll('article')[3]!.get('.pick-controls button').attributes('aria-pressed')).toBe('true');
  await wrapper
    .findAll('header button')
    .find((b) => b.text() === '撤销')!
    .trigger('click');
  await flushPromises();
  expect(records.every((a) => a.review.pickState === 'unreviewed')).toBe(true);
  expect(
    wrapper
      .findAll('article')
      .every((p) => p.get('.pick-controls button').attributes('aria-pressed') === 'false'),
  ).toBe(true);
  expect(lib.history).toEqual([]);
});
it('queues shortcut undo after a marking command that has not finished yet', async () => {
  const { wrapper, lib } = await setup();
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', bubbles: true }));
  document.dispatchEvent(
    new KeyboardEvent('keydown', { key: 'z', metaKey: true, bubbles: true, cancelable: true }),
  );
  await flushPromises();
  expect(
    invoke.mock.calls
      .filter((c) => ['apply_review_batch', 'undo_review_batch'].includes(c[0]))
      .map((c) => c[0]),
  ).toEqual(['apply_review_batch', 'undo_review_batch']);
  expect(records[0]!.review.isFavorite).toBe(false);
  expect(lib.history).toEqual([]);
  expect(records[0]!.review.revision).toBe(2);
});
