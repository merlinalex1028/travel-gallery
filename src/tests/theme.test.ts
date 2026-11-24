import { afterEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { nextTick } from 'vue';
import { useApp } from '../stores/app';

vi.mock('../services/api', () => ({ api: vi.fn(), errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({ ElMessage: { error: vi.fn(), success: vi.fn() } }));

afterEach(() => {
  vi.unstubAllGlobals();
  document.documentElement.classList.remove('dark');
  document.documentElement.style.removeProperty('font-size');
});

it('follows system appearance changes, preserves explicit choices and disposes the listener', async () => {
  const query = new EventTarget();
  const remove = vi.spyOn(query, 'removeEventListener');
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => Object.assign(query, { matches: false })),
  );
  setActivePinia(createPinia());
  const app = useApp();
  const systemChange = async (matches: boolean) => {
    query.dispatchEvent(Object.assign(new Event('change'), { matches }));
    await nextTick();
  };
  expect(app.isDark).toBe(false);
  await systemChange(true);
  expect(app.isDark).toBe(true);
  expect(document.documentElement.classList.contains('dark')).toBe(true);
  app.settings.theme = 'light';
  await nextTick();
  await systemChange(false);
  await systemChange(true);
  expect(document.documentElement.classList.contains('dark')).toBe(false);
  app.settings.theme = 'dark';
  await systemChange(false);
  expect(document.documentElement.classList.contains('dark')).toBe(true);
  app.settings.theme = 'system';
  app.settings.fontSize = 16;
  await nextTick();
  expect(document.documentElement.classList.contains('dark')).toBe(false);
  expect(document.documentElement.style.fontSize).toBe('16px');
  app.$dispose();
  expect(remove).toHaveBeenCalledWith('change', expect.any(Function));
});
