<script setup lang="ts">
import { computed, onMounted, onUnmounted, shallowRef } from 'vue';
import { api, errorMessage } from '../services/api';
import { useApp } from '../stores/app';
import { files } from '../services/format';
import type { AssetSummary, FileState } from '../services/contracts';
import SourceCard from '../features/SourceCard.vue';
import EmptyState from '../components/EmptyState.vue';
import Icon from '../components/Icon.vue';
const app = useApp(),
  items = shallowRef<AssetSummary[]>([]),
  total = shallowRef(0),
  cursor = shallowRef<string | null>(null),
  state = shallowRef<FileState>('missing'),
  loading = shallowRef(false),
  error = shallowRef('');
let loadGeneration = 0;
onUnmounted(() => {
  loadGeneration++;
});
const sections = computed(() => {
  const groups = new Map<string, AssetSummary[]>();
  for (const item of items.value) {
    const group = groups.get(item.sourceId) ?? [];
    group.push(item);
    groups.set(item.sourceId, group);
  }
  return [...groups].map(([sourceId, assets]) => ({
    sourceId,
    source: app.sources.find((source) => source.id === sourceId),
    assets,
  }));
});
async function load(more = false) {
  if (more && loading.value) return;
  const generation = ++loadGeneration,
    requestedState = state.value;
  if (!more) {
    items.value = [];
    total.value = 0;
    cursor.value = null;
  }
  error.value = '';
  loading.value = true;
  try {
    const r = await api('query_assets', {
      filter: { fileStates: [requestedState] },
      sort: { field: 'filename', direction: 'asc' },
      cursor: more ? cursor.value : null,
      pageSize: 100,
    });
    if (generation !== loadGeneration || requestedState !== state.value) return;
    items.value = more ? [...items.value, ...r.items] : r.items;
    total.value = r.total;
    cursor.value = r.nextCursor;
  } catch (e) {
    if (generation === loadGeneration && requestedState === state.value) error.value = errorMessage(e);
  } finally {
    if (generation === loadGeneration) loading.value = false;
  }
}
onMounted(() => {
  void load();
});
</script>
<template>
  <section class="standard-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">KEEP EVERY MEMORY CONNECTED</div>
        <h1>文件状态</h1>
        <p>离线与缺失分别处理，你的整理记录始终保留。</p>
      </div>
      <button class="btn" @click="load()"><Icon name="refresh" :size="15" />刷新状态</button>
    </div>
    <SourceCard v-for="source in app.sources.filter((s) => !s.isOnline)" :key="source.id" :source="source" />
    <div class="segmented wide-tabs">
      <button
        v-for="s in ['missing', 'offline', 'unsupported', 'corrupt'] as FileState[]"
        :key="s"
        :class="{ active: state === s }"
        :aria-pressed="state === s"
        @click="
          () => {
            state = s;
            load();
          }
        "
      >
        {{ files[s] }}
      </button>
    </div>
    <p class="muted small">根目录离线时不会把整盘照片判定为缺失。重新连接后，可在来源管理中重新扫描。</p>
    <p v-if="!loading && !error" class="muted small">
      当前状态 {{ total }} 项，已加载 {{ items.length }} 项。
    </p>
    <div v-if="error" class="notice warning" role="alert">
      {{ error }}<button class="text-btn" @click="load()">重试检查</button>
    </div>
    <section v-for="section in sections" :key="section.sourceId" class="status-source-group">
      <h2>{{ section.source?.displayName ?? '来源信息暂未更新' }} · 已加载 {{ section.assets.length }} 项</h2>
      <p v-if="section.source" class="path-text">{{ section.source.absolutePath }}</p>
      <div class="data-table">
        <div class="table-header">
          <span>文件</span><span>来源路径</span><span>状态</span><span>操作</span>
        </div>
        <div v-for="a in section.assets" :key="a.id" class="table-row">
          <strong>{{ a.filename }}</strong
          ><span class="path-text truncate" :title="a.relativePath">{{ a.relativePath }}</span
          ><span>{{ files[a.fileState] }}</span
          ><RouterLink :to="`/viewer/${a.id}`" class="text-btn">查看缓存</RouterLink>
        </div>
      </div>
    </section>
    <EmptyState
      v-if="!items.length"
      :title="loading ? '正在检查…' : error ? '文件状态暂不可用' : `没有${files[state]}的文件`"
      :icon="error ? 'warning' : 'success'"
      :error="!!error"
    /><button v-if="cursor" class="btn" :disabled="loading" @click="load(true)">
      载入更多（{{ items.length }}/{{ total }}）
    </button>
  </section>
</template>
<style scoped>
.status-source-group {
  margin-top: 22px;
}
.status-source-group h2 {
  font-size: 14px;
  margin-bottom: 8px;
}
.status-source-group > .path-text {
  margin-bottom: 10px;
  overflow-wrap: anywhere;
}
</style>
