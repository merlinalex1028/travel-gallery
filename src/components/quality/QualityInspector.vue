<script setup lang="ts">
import { onUnmounted, shallowRef, watch } from 'vue';
import { api, errorMessage } from '../../services/api';
import type { AssetQuality, QualityObservation } from '../../services/contracts';
import QualitySummary from './QualitySummary.vue';
const props = defineProps<{ assetId: string }>();
const open = shallowRef(false),
  analysis = shallowRef<AssetQuality | null>(null),
  loading = shallowRef(false),
  busy = shallowRef(false),
  error = shallowRef('');
let alive = true,
  generation = 0;
onUnmounted(() => {
  alive = false;
  generation++;
});
async function load() {
  const request = ++generation,
    assetId = props.assetId;
  loading.value = true;
  error.value = '';
  try {
    const result = await api('get_asset_quality', { assetId });
    if (alive && request === generation) analysis.value = result;
  } catch (e) {
    if (alive && request === generation) error.value = errorMessage(e);
  } finally {
    if (alive && request === generation) loading.value = false;
  }
}
watch(
  () => props.assetId,
  () => {
    generation++;
    analysis.value = null;
    error.value = '';
    if (open.value) void load();
  },
);
watch(open, (value) => {
  if (value) void load();
});
function toggle(e: Event) {
  open.value = (e.target as HTMLDetailsElement).open;
}
async function dismiss(observation: QualityObservation) {
  if (busy.value) return;
  const assetId = props.assetId,
    request = generation;
  busy.value = true;
  error.value = '';
  try {
    await api('set_quality_dismissed', {
      assetId,
      kind: observation.kind,
      inputKey: observation.inputKey,
      dismissed: !observation.dismissed,
      clientMutationId: crypto.randomUUID(),
    });
    if (alive && request === generation) await load();
  } catch (e) {
    if (alive && request === generation) error.value = errorMessage(e);
  } finally {
    if (alive) busy.value = false;
  }
}
</script>
<template>
  <details class="inspector-section" @toggle="toggle">
    <summary>本地检查记录</summary>
    <template v-if="open">
      <p v-if="loading" class="muted small" role="status">正在读取…</p>
      <p v-if="error" class="warning-text small" role="alert">{{ error }}</p>
      <QualitySummary v-if="analysis" :analysis="analysis" :busy="busy" @dismiss="dismiss" />
      <p v-if="analysis && !analysis.observations.length" class="muted small">
        暂无当前版本记录。<RouterLink to="/quality">设置或运行照片检查</RouterLink>
      </p>
      <p class="muted small">检查提示供人工复核，不会改变整理决定。</p>
      <button class="text-btn" :disabled="loading || busy" @click="load">重新读取</button>
    </template>
  </details>
</template>
<style scoped>
summary {
  cursor: pointer;
  font-size: 13px;
  font-weight: 600;
}
p {
  line-height: 1.8;
}
</style>
