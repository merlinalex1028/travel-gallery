<script setup lang="ts">
import { computed, onMounted, onUnmounted, watchEffect } from 'vue';
import { ElConfigProvider } from 'element-plus';
import zhCN from 'element-plus/es/locale/lang/zh-cn';
import enUS from 'element-plus/es/locale/lang/en';
import { locale, t } from './i18n';
import { useRoute, useRouter } from 'vue-router';
import { useApp } from './stores/app';
import { useLibrary } from './stores/library';
import { clearPreviewCache } from './composables/preview';
import { native } from './services/api';
import MainNav from './components/MainNav.vue';
import TopBar from './components/TopBar.vue';
import GlobalDialogs from './components/GlobalDialogs.vue';
import AppUpdates from './components/AppUpdates.vue';
import EmptyState from './components/EmptyState.vue';
import Icon from './components/Icon.vue';
const app = useApp(),
  lib = useLibrary(),
  route = useRoute(),
  router = useRouter();
const componentLocale = computed(() => (locale.value === 'en-US' ? enUS : zhCN));
watchEffect(() => {
  document.documentElement.lang = locale.value;
});
const immersive = computed(
  () => ['/cull', '/compare'].includes(route.path) || route.path.startsWith('/viewer/'),
);
let poll: ReturnType<typeof setInterval> | undefined,
  polling = false,
  priorRunning = false,
  detailRefreshPending = false,
  detailRefreshRetries = 0;
function previewState() {
  return JSON.stringify([
    app.sources.map((s) => [s.id, s.isOnline, s.lastScanAt]),
    app.jobs
      .filter(
        (j) =>
          ['SCAN', 'METADATA', 'THUMBNAIL'].includes(j.type) &&
          ['completed', 'failed', 'cancelled'].includes(j.status),
      )
      .map((j) => [j.id, j.status, j.updatedAt]),
  ]);
}
onMounted(async () => {
  await app.init();
  if (native && app.ready && !app.sources.length && route.path === '/library')
    await router.replace('/welcome');
  poll = setInterval(async () => {
    if (polling || !app.ready) return;
    polling = true;
    try {
      const hadRunning = app.running.length > 0 || priorRunning;
      const epoch = app.catalogEpoch,
        previews = previewState();
      await app.refresh();
      const previewsChanged = (hadRunning && !app.running.length) || previews !== previewState();
      if (previewsChanged) clearPreviewCache();
      if (previewsChanged || epoch !== app.catalogEpoch) {
        // A small watched-directory scan can start and finish between polls.
        detailRefreshPending = true;
        detailRefreshRetries = 1;
        if (!immersive.value) await lib.search();
      } else if (
        app.running.some((j) => j.type === 'SCAN') &&
        lib.items.length <= 120 &&
        ['/library', '/filter'].includes(route.path)
      )
        await lib.search();
      if (detailRefreshPending && lib.detail?.id === lib.focusId) {
        const refreshed = await lib.inspect(lib.detail.id, { background: true });
        detailRefreshPending = !refreshed && detailRefreshRetries-- > 0;
      }
      priorRunning = app.running.length > 0;
    } catch {
      /* Keep current content during a transient polling error. */
    } finally {
      polling = false;
    }
  }, 1500);
});
onUnmounted(() => {
  if (poll) clearInterval(poll);
});
</script>
<template>
  <ElConfigProvider :locale="componentLocale"
    ><div class="app-shell" :class="{ immersive, 'nav-collapsed': app.navCollapsed }">
      <template v-if="!immersive"
        ><MainNav />
        <div class="main-shell">
          <TopBar />
          <main class="main-content">
            <EmptyState
              v-if="app.error"
              icon="warning"
              :title="t('app.libraryUnavailable')"
              :description="app.error"
              :action="t('common.retry')"
              error
              @action="app.init"
            />
            <div v-else-if="!app.ready" class="app-loading">
              <Icon class="spin" name="loader" />{{ t('app.opening') }}
            </div>
            <RouterView v-else />
          </main>
          <footer class="statusbar">
            <span
              ><span class="tiny-dot green" />{{ t('app.localCatalog') }}
              <span class="status-separator">|</span>
              {{ t('app.mediaCount', { count: app.total.toLocaleString(locale) }) }}</span
            ><RouterLink v-if="app.running.length" to="/jobs" class="status-task"
              ><Icon name="loader" class="spin" :size="13" />{{
                t('app.jobCount', { count: app.running.length })
              }}</RouterLink
            ><span v-else><Icon name="shield" :size="12" />{{ t('app.localRecords') }}</span>
          </footer>
        </div></template
      ><RouterView v-else /><GlobalDialogs /><AppUpdates /></div
  ></ElConfigProvider>
</template>
