import { defineStore } from 'pinia';
import { computed, reactive, shallowRef, watch } from 'vue';
import { api, errorMessage } from '../services/api';
import { settingsWithDefaults } from '../services/settings';
import { ElMessage } from 'element-plus';
import type {
  AppSettings,
  JobSummary,
  SavedFilter,
  SourceRoot,
  Trip,
  SelectionTarget,
  NamingRule,
} from '../services/contracts';

export const useApp = defineStore('app', () => {
  const ready = shallowRef(false),
    catalogEpoch = shallowRef(0),
    error = shallowRef(''),
    importOpen = shallowRef(false),
    exportOpen = shallowRef(false),
    renameOpen = shallowRef(false),
    renameJobId = shallowRef(''),
    exportDraft = shallowRef<{ target: SelectionTarget; count: number; naming: NamingRule } | null>(null),
    helpOpen = shallowRef(false),
    navCollapsed = shallowRef(false);
  const sources = shallowRef<SourceRoot[]>([]),
    jobs = shallowRef<JobSummary[]>([]),
    trips = shallowRef<Trip[]>([]),
    saved = shallowRef<SavedFilter[]>([]);
  const settings = reactive<AppSettings>(settingsWithDefaults());
  const total = computed(() =>
    sources.value.filter((s) => s.isEnabled).reduce((n, s) => n + s.totalAssets, 0),
  );
  const running = computed(() =>
    jobs.value.filter((j) => ['queued', 'running', 'paused'].includes(j.status)),
  );
  const tags = shallowRef<{ id: string; name: string; color: string | null; count: number }[]>([]);
  const watcher = shallowRef({
    active: false,
    watchedRoots: 0,
    pollingRoots: 0,
    lastError: null as string | null,
  });
  const power = shallowRef({ onBattery: null as boolean | null });
  async function refresh() {
    const results = await Promise.all([
      api('list_sources', undefined),
      api('list_jobs', { cursor: null, pageSize: 100 }),
      api('list_tags', undefined),
      api('list_trips', undefined),
      api('list_saved_filters', undefined),
      api('get_app_status', undefined),
    ]);
    sources.value = results[0];
    jobs.value = results[1].items;
    tags.value = results[2];
    trips.value = results[3];
    saved.value = results[4];
    watcher.value = results[5].watcher;
    catalogEpoch.value = results[5].catalogEpoch;
    power.value = results[5].power ?? { onBattery: null };
  }
  async function init() {
    try {
      Object.assign(settings, settingsWithDefaults(await api('get_settings', undefined)));
      await refresh();
      ready.value = true;
      error.value = '';
    } catch (e) {
      error.value = errorMessage(e);
    }
  }
  let settingsQueue: Promise<unknown> = Promise.resolve();
  function persist() {
    const value = { ...settings };
    const task = settingsQueue.catch(() => {}).then(() => api('save_settings', value));
    settingsQueue = task;
    return task;
  }
  async function perform<T>(action: () => Promise<T>, message?: string): Promise<T | undefined> {
    try {
      const result = await action();
      if (message) ElMessage.success(message);
      return result;
    } catch (e) {
      ElMessage.error(errorMessage(e));
      return undefined;
    }
  }
  watch(
    () => [settings.theme, settings.fontSize],
    () => {
      document.documentElement.classList.toggle(
        'dark',
        settings.theme === 'dark' ||
          (settings.theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches),
      );
      document.documentElement.style.fontSize = `${settings.fontSize}px`;
    },
    { immediate: true },
  );
  return {
    ready,
    catalogEpoch,
    error,
    importOpen,
    exportOpen,
    renameOpen,
    renameJobId,
    exportDraft,
    helpOpen,
    navCollapsed,
    sources,
    jobs,
    trips,
    saved,
    tags,
    watcher,
    power,
    settings,
    total,
    running,
    refresh,
    init,
    persist,
    perform,
  };
});
