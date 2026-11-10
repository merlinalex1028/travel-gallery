import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { createMemoryHistory, createRouter } from 'vue-router';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import LibraryPage from '../pages/LibraryPage.vue';
import { useLibrary } from '../stores/library';
import { useApp } from '../stores/app';
import type { AssetSummary } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({
  ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
  ElMessageBox: { prompt: vi.fn() },
}));
let wrapper: VueWrapper | undefined;
const assets = ['a', 'b'].map((id) => ({ ...golden.assetDetail, id }) as AssetSummary);
beforeEach(() => {
  document.body.innerHTML = '';
  setActivePinia(createPinia());
  invoke.mockReset();
  invoke.mockImplementation(async (name: string) => {
    if (name === 'query_assets')
      return { items: assets, total: 50000, nextCursor: 'next', queryFingerprint: 'query' };
    if (name === 'get_asset') return assets[0];
    if (name === 'apply_review_batch')
      return { changeBatchId: 'changed', affectedCount: 1, unchangedCount: 0, conflicts: [] };
    throw new Error(name);
  });
});
afterEach(() => {
  wrapper?.unmount();
  wrapper = undefined;
  document.body.innerHTML = '';
});
async function openLibrary() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/library', component: LibraryPage }],
  });
  await router.push('/library');
  useApp().settings.showInspector = false;
  wrapper = mount(LibraryPage, {
    attachTo: document.body,
    global: {
      plugins: [router],
      stubs: {
        Icon: true,
        LibraryToolbar: true,
        FilterPanel: true,
        SelectionBar: true,
        Inspector: true,
        MediaGrid: {
          props: ['items'],
          template:
            '<button class="test-card" @contextmenu.prevent="$emit(\'context\', items[0], $event)">Photo</button>',
        },
      },
    },
  });
  await flushPromises();
  const lib = useLibrary();
  lib.selected = new Set(['a']);
  return lib;
}
function selectAll(target: EventTarget = document, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent('keydown', {
    key: 'a',
    metaKey: true,
    bubbles: true,
    cancelable: true,
    composed: true,
    ...init,
  });
  target.dispatchEvent(event);
  return event;
}

describe('library keyboard scope', () => {
  it('selects only the loaded records and removes its listener when leaving', async () => {
    const lib = await openLibrary();
    expect(selectAll().defaultPrevented).toBe(true);
    expect([...lib.selected]).toEqual(['a', 'b']);
    expect(lib.count).toBe(2);
    wrapper!.unmount();
    wrapper = undefined;
    lib.selected = new Set(['a']);
    expect(selectAll().defaultPrevented).toBe(false);
    expect([...lib.selected]).toEqual(['a']);
  });
  it('preserves a frozen selection while a visible modal owns keyboard input', async () => {
    const lib = await openLibrary();
    lib.snapshot = {
      id: 'frozen',
      count: 50000,
      createdAt: 1,
      expiresAt: 9999999999999,
      queryFingerprint: 'query',
    };
    const overlay = document.createElement('div');
    overlay.className = 'el-overlay-dialog';
    document.body.append(overlay);
    expect(selectAll().defaultPrevented).toBe(false);
    expect(lib.target).toEqual({ kind: 'snapshot', snapshotId: 'frozen', excludeAssetIds: [] });
    expect(lib.count).toBe(50000);
    overlay.style.display = 'none';
    expect(selectAll(document, { key: 'A', ctrlKey: true, metaKey: false }).defaultPrevented).toBe(true);
    expect([...lib.selected]).toEqual(['a', 'b']);
  });
  it('leaves editable, select, menu, shadow and already consumed shortcuts alone', async () => {
    const lib = await openLibrary();
    for (const html of [
      '<select><option>Value</option></select>',
      '<div contenteditable="true"><span>Text</span></div>',
      '<div role="menu"><button>Action</button></div>',
      '<div role="listbox"><div role="option">Value</div></div>',
    ]) {
      const host = document.createElement('div');
      host.innerHTML = html;
      document.body.append(host);
      const event = selectAll(host.querySelector('span,button,[role="option"],select')!);
      expect(event.defaultPrevented).toBe(false);
      expect([...lib.selected]).toEqual(['a']);
      host.remove();
    }
    const host = document.createElement('div');
    const input = document.createElement('input');
    host.attachShadow({ mode: 'open' }).append(input);
    document.body.append(host);
    expect(selectAll(input).defaultPrevented).toBe(false);
    expect([...lib.selected]).toEqual(['a']);
    const consumed = new KeyboardEvent('keydown', { key: 'a', metaKey: true, cancelable: true });
    consumed.preventDefault();
    document.dispatchEvent(consumed);
    selectAll(document, { isComposing: true });
    selectAll(document, { altKey: true });
    selectAll(document, { shiftKey: true });
    selectAll(document, { repeat: true });
    expect([...lib.selected]).toEqual(['a']);
    expect(selectAll(document, { metaKey: false, ctrlKey: true }).defaultPrevented).toBe(true);
  });
  it('keeps an open context menu in its own scope and Escape restores photo shortcuts', async () => {
    const lib = await openLibrary();
    await wrapper!.get('.test-card').trigger('contextmenu');
    await flushPromises();
    const menu = document.querySelector<HTMLElement>('[role="menu"]')!;
    expect(menu).not.toBeNull();
    expect(menu.contains(document.activeElement)).toBe(true);
    selectAll();
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'x', bubbles: true }));
    await flushPromises();
    expect([...lib.selected]).toEqual(['a']);
    expect(invoke.mock.calls.some(([name]) => name === 'apply_review_batch')).toBe(false);
    document.activeElement!.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }),
    );
    await flushPromises();
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(selectAll().defaultPrevented).toBe(true);
  });
});
