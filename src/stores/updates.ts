import { defineStore } from 'pinia';
import { computed, shallowRef } from 'vue';
import { updater, type AppUpdate } from '../services/updater';
import { errorMessage } from '../services/api';
import { version } from '../../package.json';

export const useUpdates = defineStore('updates', () => {
  const supported = shallowRef(false),
    initialized = shallowRef(false),
    currentVersion = shallowRef(version);
  const phase = shallowRef<
    'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'installing' | 'current'
  >('idle');
  const candidate = shallowRef<AppUpdate | null>(null),
    error = shallowRef(''),
    lastChecked = shallowRef<number | null>(null);
  const downloaded = shallowRef(0),
    total = shallowRef<number | null>(null),
    dismissed = shallowRef('');
  const busy = computed(() => ['checking', 'downloading', 'installing'].includes(phase.value));
  const progress = computed(() =>
    total.value ? Math.min(100, Math.floor((downloaded.value / total.value) * 100)) : null,
  );
  let initialization: Promise<void> | undefined;
  function init() {
    return (initialization ??= (async () => {
      try {
        const info = await updater.support();
        supported.value = info.supported;
        currentVersion.value = info.version;
      } catch {
        supported.value = false;
      }
      initialized.value = true;
    })());
  }
  async function check() {
    await init();
    if (!supported.value || busy.value || phase.value === 'ready') return;
    phase.value = 'checking';
    error.value = '';
    try {
      candidate.value = await updater.check();
      lastChecked.value = Date.now();
      phase.value = candidate.value ? 'available' : 'current';
    } catch (e) {
      error.value = errorMessage(e);
      phase.value = candidate.value ? 'available' : 'idle';
    }
  }
  async function download() {
    if (!supported.value || !candidate.value || busy.value || phase.value === 'ready') return;
    phase.value = 'downloading';
    error.value = '';
    downloaded.value = 0;
    total.value = null;
    try {
      await updater.download((value) => {
        downloaded.value = value.downloaded;
        total.value = value.total;
      });
      phase.value = 'ready';
    } catch (e) {
      error.value = errorMessage(e);
      phase.value = 'available';
    }
  }
  async function install() {
    if (!supported.value || phase.value !== 'ready') return;
    phase.value = 'installing';
    error.value = '';
    try {
      await updater.install();
    } catch (e) {
      error.value = errorMessage(e);
      phase.value = 'ready';
    }
  }
  return {
    supported,
    initialized,
    currentVersion,
    phase,
    candidate,
    error,
    lastChecked,
    downloaded,
    total,
    dismissed,
    busy,
    progress,
    init,
    check,
    download,
    install,
  };
});
