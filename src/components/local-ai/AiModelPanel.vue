<script setup lang="ts">
import { computed, onUnmounted, shallowRef } from 'vue';
import { api, errorMessage, isDemo } from '../../services/api';
import { bytes } from '../../services/format';
import type { AiStatus, AiModelInfo } from '../../services/contracts';
const props = defineProps<{
  status: AiStatus | null;
  actionBusy: boolean;
  active: boolean;
  modelBusy: boolean;
  kind?: 'content' | 'eyes';
}>();
const eyes = computed(() => props.kind === 'eyes');
const emit = defineEmits<{ import: []; toggle: []; remove: [] }>();
let alive = true;
onUnmounted(() => {
  alive = false;
});
const infoOpen = shallowRef(false),
  info = shallowRef<AiModelInfo | null>(null),
  infoLoading = shallowRef(false),
  infoError = shallowRef('');

async function loadInfo() {
  infoOpen.value = true;
  if (info.value || infoLoading.value) return;
  infoLoading.value = true;
  infoError.value = '';
  try {
    const value = eyes.value
      ? await api('get_eyes_model_info', undefined)
      : await api('get_ai_model_info', undefined);
    if (alive) info.value = value;
  } catch (error) {
    if (alive) infoError.value = errorMessage(error);
  } finally {
    if (alive) infoLoading.value = false;
  }
}
</script>
<template>
  <section class="settings-card ai-model-card">
    <div class="row">
      <div>
        <h2>可选离线模型</h2>
        <p>
          {{ status?.profileName ?? (eyes ? 'MediaPipe Face Landmarker V1' : 'Google SigLIP 2 Base') }} ·
          Apache-2.0
        </p>
      </div>
      <span class="state-pill">{{
        status?.modelAvailable ? '已导入' : status?.modelFolderPresent ? '需要重新校验' : '未导入'
      }}</span>
    </div>
    <p>
      默认关闭。通过系统选择器导入完整离线模型包后，再主动启用。图像与分析仅在本机处理，应用不下载模型或上传照片。
    </p>
    <p class="muted small">
      模型包约 {{ status ? bytes(status.modelBytes) : eyes ? '9.54 MB' : '1.43 GB' }}；
      <template v-if="eyes">使用 1 个 CPU 线程，仅分析照片预览，每张最多 10 张人脸。</template>
      <template v-else>本机实测推理进程峰值约 2.2 GiB。推理最多 2 个 CPU 线程，视频按静帧参与检索。</template>
      单张处理，空闲约 60 秒后释放。
    </p>
    <div class="ai-model-actions">
      <button class="btn" :disabled="isDemo || actionBusy || active" @click="emit('import')">
        {{ actionBusy ? '处理中…' : status?.modelAvailable ? '重新导入模型包' : '导入离线模型包' }}</button
      ><button
        class="btn primary"
        :disabled="
          isDemo || !status || (!status.modelAvailable && !status.enabled) || actionBusy || modelBusy
        "
        @click="emit('toggle')"
      >
        {{ status?.enabled ? '关闭并释放模型' : eyes ? '启用闭眼提示' : '启用本地 AI' }}</button
      ><button class="text-btn" @click="loadInfo">模型说明与许可</button
      ><button
        v-if="status?.modelFolderPresent || status?.indexedCount"
        class="text-btn danger-text"
        :disabled="actionBusy || active"
        @click="emit('remove')"
      >
        {{ eyes ? '移除模型和闭眼记录' : '移除模型和 AI 索引' }}
      </button>
    </div>
    <p v-if="status?.enabled" class="muted small" role="status">
      {{
        status.runtime === 'loading'
          ? '正在加载本地模型…'
          : status.runtime === 'loaded'
            ? '模型已加载；关闭此项可立即释放推理进程。'
            : '模型空闲，开始分析时按需加载。'
      }}
    </p>
    <p v-if="isDemo" class="notice">浏览器演示不导入模型或执行推理，请使用原生桌面应用。</p>
  </section>
  <ElDialog v-model="infoOpen" title="本地模型说明与许可" width="760px"
    ><p v-if="infoLoading" role="status">正在读取随应用附带的说明…</p>
    <p v-if="infoError" class="notice warning" role="alert">
      {{ infoError }} <button class="text-btn" @click="loadInfo">重试</button>
    </p>
    <template v-if="info"
      ><p>
        {{ info.profile.repository }} · 版本 {{ info.profile.revision }} ·
        {{ info.profile.license }}
      </p>
      <p>
        离线包可在联网电脑上使用项目提供的准备脚本获取，再将整个文件夹传到离线电脑，通过「导入离线模型包」选择。模型和推理进程可独立释放，不影响手工整理。
      </p>
      <h3>{{ eyes ? '模型与提示规则' : '作者模型说明' }}</h3>
      <pre class="ai-model-notice" tabindex="0">{{ info.card }}</pre>
      <h3>许可正文</h3>
      <pre class="ai-model-notice" tabindex="0">{{ info.licenseText }}</pre>
    </template></ElDialog
  >
</template>
<style scoped>
.ai-model-card {
  margin-bottom: 24px;
}
.ai-model-card h2 {
  margin: 0 0 8px;
  font-size: 20px;
}
.ai-model-card p {
  line-height: 1.8;
}
.ai-model-actions {
  display: flex;
  gap: 14px;
  flex-wrap: wrap;
  align-items: center;
  margin-top: 20px;
}
.ai-model-notice {
  max-height: 34vh;
  overflow: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  background: var(--surface-raised);
  padding: 16px;
  border: var(--bw) solid var(--line);
  border-radius: var(--r-sm);
  font-size: 12px;
  line-height: 1.6;
}
</style>
