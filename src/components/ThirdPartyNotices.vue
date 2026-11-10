<script setup lang="ts">
import { shallowRef } from 'vue';
const open = shallowRef(false),
  text = shallowRef(''),
  error = shallowRef(''),
  loading = shallowRef(false);
async function show() {
  open.value = true;
  if (text.value || loading.value) return;
  error.value = '';
  loading.value = true;
  try {
    const response = await fetch(new URL('third-party-notices.txt', window.location.href));
    if (!response.ok) throw new Error('unavailable');
    const value = await response.text();
    if (!value.trim()) throw new Error('empty');
    text.value = value;
  } catch {
    error.value = '许可文件暂时无法读取，请重试。';
  } finally {
    loading.value = false;
  }
}
</script>
<template>
  <button class="text-btn" @click="show">第三方许可</button>
  <ElDialog v-model="open" title="第三方许可" width="min(960px, calc(100vw - 36px))" class="notices-dialog">
    <p class="muted small">开源组件、许可正文与来源随应用保存在本机。</p>
    <p v-if="loading" role="status">正在读取许可…</p>
    <div v-else-if="error" role="alert">
      <p>{{ error }}</p>
      <button class="btn" @click="show">重试</button>
    </div>
    <pre v-else class="notices-text" tabindex="0" aria-label="第三方许可正文">{{ text }}</pre>
  </ElDialog>
</template>
