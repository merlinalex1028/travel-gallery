<script setup lang="ts">
import { locale, tr } from '../i18n';

import { onMounted, shallowRef } from 'vue';
import { ElMessageBox } from 'element-plus';
import { useUpdates } from '../stores/updates';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { errorMessage } from '../services/api';
import { bytes } from '../services/format';
import Icon from './Icon.vue';
const updates = useUpdates(),
  app = useApp(),
  lib = useLibrary();
const saving = shallowRef(false),
  confirming = shallowRef(false),
  localError = shallowRef('');
onMounted(() => updates.init());
async function setAutomatic(value: boolean) {
  const previous = app.settings.autoCheckUpdates;
  saving.value = true;
  localError.value = '';
  app.settings.autoCheckUpdates = value;
  try {
    await app.persist();
  } catch (e) {
    app.settings.autoCheckUpdates = previous;
    localError.value = errorMessage(e);
  } finally {
    saving.value = false;
  }
}
async function install() {
  if (confirming.value || updates.phase !== 'ready') return;
  if (app.running.length || lib.pending) {
    localError.value = tr('请先完成或取消所有后台任务，并等待整理修改保存完成。');
    return;
  }
  confirming.value = true;
  localError.value = '';
  try {
    const confirmed = await ElMessageBox.confirm(
      tr('安装更新后将关闭并重新打开旅图。照片和本地资料库会保留，请先保存当前编辑。'),
      tr('更新至 v{0}', { 0: updates.candidate?.version }),
      { confirmButtonText: tr('安装并重启'), cancelButtonText: tr('稍后'), type: 'info' },
    )
      .then(() => true)
      .catch(() => false);
    if (!confirmed) return;
    if (app.running.length || lib.pending) {
      localError.value = tr('仍有任务或修改在保存，请完成后重试。');
      return;
    }
    await app.persist();
    await updates.install();
  } catch (e) {
    localError.value = errorMessage(e);
  } finally {
    confirming.value = false;
  }
}
</script>
<template>
  <div id="app-updates" class="settings-card update-card" :aria-busy="updates.busy">
    <h3><Icon name="refresh" />{{ tr('应用更新') }}</h3>
    <div class="setting-row">
      <div>
        <strong>{{ tr('自动检查更新') }}</strong>
        <p>{{ tr('默认关闭。开启后在启动时及每 4 小时检查正式版；发现更新后由你决定下载和安装。') }}</p>
      </div>
      <ElSwitch
        :model-value="app.settings.autoCheckUpdates"
        :disabled="!updates.supported || saving || updates.busy"
        :aria-label="tr('自动检查更新')"
        @update:model-value="setAutomatic(Boolean($event))"
      />
    </div>
    <div class="setting-row">
      <div>
        <strong>{{ tr('当前版本 v') }}{{ updates.currentVersion }}</strong>
        <p v-if="!updates.initialized">{{ tr('正在读取更新配置…') }}</p>
        <p v-else-if="!updates.supported">
          {{
            tr('浏览器、开发和验收版本不联网检查或安装更新。支持 macOS Apple Silicon 和 Windows x64 发行版。')
          }}
        </p>
        <p v-else-if="updates.phase === 'checking'">{{ tr('正在检查新版本…') }}</p>
        <p v-else-if="updates.phase === 'current'">{{ tr('当前已是最新正式版。') }}</p>
        <p v-else>{{ tr('可随时手动检查；预发布体验版请从官网下载。') }}</p>
        <p v-if="updates.lastChecked">
          {{ tr('上次检查：') }}{{ new Date(updates.lastChecked).toLocaleString(locale) }}
        </p>
      </div>
      <button
        class="btn small-btn"
        :disabled="!updates.supported || updates.busy || updates.phase === 'ready'"
        @click="updates.check"
      >
        <Icon
          :name="updates.phase === 'checking' ? 'loader' : 'refresh'"
          :class="{ spin: updates.phase === 'checking' }"
          :size="15"
        />{{ tr('检查更新') }}
      </button>
    </div>
    <div v-if="updates.candidate" class="update-release">
      <div class="setting-row">
        <div>
          <span class="eyebrow">NEW VERSION</span>
          <h4>{{ tr('旅图 v') }}{{ updates.candidate.version }}</h4>
        </div>
        <button v-if="updates.phase === 'available'" class="btn primary small-btn" @click="updates.download">
          <Icon name="download" :size="15" />{{ tr('下载更新') }}
        </button>
        <button
          v-if="updates.phase === 'ready'"
          class="btn primary small-btn"
          :disabled="confirming || !!app.running.length || !!lib.pending"
          @click="install"
        >
          {{ tr('安装并重启') }}
        </button>
      </div>
      <p v-if="updates.candidate.notes" class="update-notes">{{ updates.candidate.notes }}</p>
      <div v-if="updates.phase === 'downloading'" role="status">
        <progress :value="updates.progress ?? undefined" max="100" :aria-label="tr('更新下载进度')" />
        <p>
          {{ bytes(updates.downloaded)
          }}<template v-if="updates.total"> / {{ bytes(updates.total) }} · {{ updates.progress }}%</template
          ><template v-else> {{ tr('· 正在下载…') }}</template>
        </p>
      </div>
      <p v-if="updates.phase === 'ready'">
        {{ tr('下载完成，签名已验证。安装前请完成或取消后台任务；退出应用会清除本次下载。') }}
      </p>
      <p v-if="updates.phase === 'ready' && app.running.length">
        {{ tr('仍有未完成任务（包括暂停任务），完成或取消后可安装。') }}
      </p>
      <p v-if="updates.phase === 'installing'" role="status">{{ tr('正在安装并重启，请勿关闭应用…') }}</p>
    </div>
    <p v-if="updates.error || localError" class="update-error" role="alert">
      {{ localError || updates.error }}
    </p>
    <p class="small muted">
      {{
        tr(
          '检查与下载通过 GitHub 及其附件服务联网，不发送照片、文件路径或资料库内容。更新包会验证签名；系统仍可能要求安装授权。macOS 请先将应用移入「应用程序」。',
        )
      }}
    </p>
  </div>
</template>
<style scoped>
.update-release {
  background: var(--accent-soft);
  border: 1px solid var(--border);
  border-radius: 14px;
  padding: 0 18px 14px;
  margin-bottom: 16px;
}
.update-release h4 {
  margin: 4px 0 0;
  font-size: 18px;
}
.update-notes {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  max-height: 180px;
  overflow-y: auto;
}
.update-release progress {
  width: 100%;
  accent-color: var(--accent);
}
.update-error {
  color: var(--danger);
}
</style>
