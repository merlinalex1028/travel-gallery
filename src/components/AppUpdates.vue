<script setup lang="ts">
import { tr } from '../i18n';

import { nextTick, onMounted, onUnmounted, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useUpdates } from '../stores/updates';
import Icon from './Icon.vue';
const app = useApp(),
  updates = useUpdates();
const router = useRouter();
async function openSettings() {
  await router.push('/settings');
  await nextTick();
  document.getElementById('app-updates')?.scrollIntoView({ block: 'center' });
}
let timer: ReturnType<typeof setInterval> | undefined;
// Startup and opt-in checks run from one app-wide owner, not on each settings visit.
watch(
  () => app.ready && app.settings.autoCheckUpdates,
  async (enabled) => {
    if (enabled) await updates.check();
  },
  { immediate: true },
);
onMounted(() => {
  void updates.init();
  timer = setInterval(
    () => {
      if (app.ready && app.settings.autoCheckUpdates) void updates.check();
    },
    4 * 60 * 60 * 1000,
  );
});
onUnmounted(() => clearInterval(timer));
</script>
<template>
  <aside
    v-if="
      updates.candidate &&
      updates.dismissed !== updates.candidate.version &&
      !['downloading', 'installing'].includes(updates.phase)
    "
    class="update-notice"
    aria-live="polite"
  >
    <Icon name="download" :size="20" />
    <div>
      <strong
        >{{ updates.phase === 'ready' ? tr('更新已准备好') : tr('发现新版本') }} · v{{
          updates.candidate.version
        }}</strong
      >
      <p>{{ tr('在设置中') }}{{ updates.phase === 'ready' ? tr('安装并重启') : tr('查看与下载更新') }}</p>
    </div>
    <button class="btn small-btn" @click="openSettings">{{ tr('查看') }}</button>
    <button
      class="icon-btn"
      :aria-label="tr('稍后处理更新')"
      @click="updates.dismissed = updates.candidate.version"
    >
      <Icon name="x" :size="16" />
    </button>
  </aside>
  <ElDialog
    :model-value="updates.phase === 'installing'"
    :title="tr('正在安装更新')"
    :show-close="false"
    :close-on-click-modal="false"
    :close-on-press-escape="false"
    :width="440"
    align-center
    append-to-body
  >
    <div class="update-installing" role="status">
      <Icon name="loader" class="spin" :size="28" /><strong>{{ tr('正在安装并重启旅图…') }}</strong>
      <p>{{ tr('请勿关闭应用。照片与本地资料库会保留。') }}</p>
    </div></ElDialog
  >
</template>
<style scoped>
.update-notice {
  position: fixed;
  right: 24px;
  bottom: 44px;
  z-index: 1500;
  display: flex;
  align-items: center;
  gap: 14px;
  max-width: calc(100vw - 48px);
  padding: 16px 18px;
  border: 1px solid var(--border);
  border-radius: 16px;
  background: var(--surface);
  box-shadow: var(--shadow-lg);
  color: var(--text);
}
.update-notice p {
  margin: 5px 0 0;
  font-size: 12px;
  color: var(--muted);
}
.update-installing {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 18px;
  color: var(--text);
  text-align: center;
  padding: 28px 12px;
}
</style>
