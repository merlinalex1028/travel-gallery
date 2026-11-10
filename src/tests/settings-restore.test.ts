import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { useApp } from '../stores/app';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({ ElMessage: { error: vi.fn(), success: vi.fn() } }));
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
});
function returnedSettings(settings: Record<string, unknown>) {
  invoke.mockImplementation(async (name: string) => {
    if (name === 'get_settings') return settings;
    if (name === 'list_jobs') return { items: [] };
    if (name === 'get_app_status')
      return {
        catalogEpoch: 1,
        watcher: { active: false, watchedRoots: 0, pollingRoots: 0, lastError: null },
      };
    if (['list_sources', 'list_tags', 'list_trips', 'list_saved_filters'].includes(name)) return [];
    throw new Error(name);
  });
}
it('restores defaults when the selected backup has no saved preferences', async () => {
  const app = useApp();
  Object.assign(app.settings, {
    autoAdvance: false,
    density: 300,
    fontSize: 18,
    cacheLimitGb: 12,
    showFilmstrip: false,
  });
  returnedSettings({});
  await app.init();
  expect(app.ready).toBe(true);
  expect(app.settings).toMatchObject({
    autoAdvance: true,
    density: 220,
    fontSize: 14,
    cacheLimitGb: 5,
    showFilmstrip: true,
  });
  expect(invoke.mock.calls.some(([name]) => name === 'save_settings')).toBe(false);
});
it('fills legacy missing fields while preserving the preferences that were actually backed up', async () => {
  const app = useApp();
  Object.assign(app.settings, { autoAdvance: false, cacheLimitGb: 12, prefetch: 0 });
  returnedSettings({ theme: 'dark', fontSize: 17 });
  await app.init();
  expect(app.settings).toMatchObject({
    theme: 'dark',
    fontSize: 17,
    autoAdvance: true,
    cacheLimitGb: 5,
    prefetch: 2,
  });
});
