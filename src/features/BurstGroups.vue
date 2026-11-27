<script setup lang="ts">
import { tr } from '../i18n';

import CustomSelect from '../components/CustomSelect.vue';
import { computed, onMounted, onUnmounted, shallowRef, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api, errorMessage, isDemo } from '../services/api';
import type { AssetFilter, BurstGroup, Commands } from '../services/contracts';
import MediaThumbnail from '../components/MediaThumbnail.vue';
import EmptyState from '../components/EmptyState.vue';
import Icon from '../components/Icon.vue';

const app = useApp(),
  lib = useLibrary(),
  router = useRouter();
const source = shallowRef(''),
  gap = shallowRef(2),
  loading = shallowRef(false),
  opening = shallowRef(''),
  error = shallowRef('');
const result = shallowRef<Commands['query_burst_groups']['result'] | null>(null);
const filter = computed<AssetFilter>(() => (source.value ? { sourceIds: [source.value] } : {}));
let generation = 0,
  destroyed = false;
async function load(more = false) {
  const seq = ++generation;
  loading.value = true;
  error.value = '';
  try {
    const next = await api('query_burst_groups', {
      filter: filter.value,
      gapSeconds: gap.value,
      cursor: more ? (result.value?.nextCursor ?? null) : null,
      pageSize: 20,
    });
    if (destroyed || seq !== generation) return;
    result.value = more && result.value ? { ...next, items: [...result.value.items, ...next.items] } : next;
  } catch (e) {
    if (!destroyed && seq === generation) error.value = errorMessage(e);
  } finally {
    if (!destroyed && seq === generation) loading.value = false;
  }
}
async function open(group: BurstGroup) {
  opening.value = group.id;
  try {
    await app.perform(async () => {
      const membership = await api('create_burst_snapshot', {
        filter: filter.value,
        gapSeconds: gap.value,
        groupId: group.id,
      });
      if (destroyed) return;
      lib.clearSelection();
      lib.setFilter({ selectionSnapshotId: membership.id });
      Object.assign(lib.sort, { field: 'captureUtcMs', direction: 'asc' });
      await router.push('/filter?showResults=1');
    });
  } finally {
    opening.value = '';
  }
}
watch([source, gap], () => {
  result.value = null;
  void load();
});
onMounted(() => {
  void load();
});
onUnmounted(() => {
  destroyed = true;
  generation++;
});
</script>
<template>
  <div class="burst-groups">
    <div class="notice">
      <Icon name="clock" /><span
        >{{ isDemo ? tr('演示拍摄组仅展示交互。') : tr('按拍摄时间整理连续照片。')
        }}{{
          tr(
            '同来源、父文件夹、机型、日期和时区内，相邻照片不超过设定间隔，每组最多跨 30 秒。拍摄接近不代表视觉相似。',
          )
        }}</span
      >
    </div>
    <div class="duplicate-controls">
      <CustomSelect
        v-model="source"
        :disabled="!!opening"
        :aria-label="tr('拍摄组来源')"
        :options="[
          { value: '', label: tr('全部已启用来源') },
          ...app.sources.filter((s) => s.isEnabled).map((s) => ({ value: s.id, label: s.displayName })),
        ]"
      />
      <label
        >{{ tr('相邻间隔') }}
        <CustomSelect
          v-model="gap"
          :disabled="!!opening"
          :aria-label="tr('拍摄组相邻间隔')"
          :options="[
            { value: 1, label: tr('1 秒') },
            { value: 2, label: tr('2 秒') },
            { value: 5, label: tr('5 秒') },
            { value: 10, label: tr('10 秒') },
          ]"
      /></label>
      <button class="btn small-btn" :disabled="loading || !!opening" @click="load()">
        <Icon name="refresh" :size="14" />{{ tr('刷新拍摄组') }}
      </button>
    </div>
    <p class="muted small">
      {{
        tr(
          '至少 2 张照片组成一组；仅使用有机型信息的真实拍摄时间或手动修正时间，文件修改时间不参与。未知时区只在同一天内比较。已确认配套文件合并为一张。',
        )
      }}
    </p>
    <div v-if="result" class="section-title">
      <strong>{{ result.total }} {{ tr('个拍摄组') }}</strong
      ><span class="muted small">{{ tr('检查了') }} {{ result.examinedCount }} {{ tr('张候选照片') }}</span>
    </div>
    <div v-if="result?.truncated" class="notice warning">
      {{ tr('候选超过 100,000 张，仅显示本次检查范围内的完整拍摄组。请选择一个来源缩小范围。') }}
    </div>
    <div v-if="error" role="alert" class="notice warning">
      {{ error }} <button class="text-btn" @click="load()">{{ tr('刷新重试') }}</button>
    </div>
    <article v-for="group in result?.items ?? []" :key="group.id" class="duplicate-group">
      <header>
        <div>
          <strong>{{ group.startLocal }}</strong
          ><span class="muted">{{ group.memberCount }} {{ tr('张 ·') }} {{ group.cameraModel }}</span
          ><span class="muted small"
            >{{ tr('至') }} {{ group.endLocal.slice(11) }} ·
            {{ group.captureTzMin === null ? tr('时区未知') : tr('同一时区') }}</span
          >
        </div>
        <button class="btn small-btn" :disabled="!!opening || loading" @click="open(group)">
          {{ opening === group.id ? tr('正在打开…') : tr('打开整组挑选') }}
        </button>
      </header>
      <div class="burst-preview">
        <div v-for="asset in group.members" :key="asset.id">
          <MediaThumbnail :asset-id="asset.id" :alt="asset.filename" /><span
            class="truncate"
            :title="asset.filename"
            >{{ asset.filename }}</span
          >
        </div>
      </div>
      <p class="muted small burst-folder">
        {{ app.sources.find((s) => s.id === group.sourceId)?.displayName ?? tr('来源') }} /
        {{ group.folder || tr('根目录')
        }}<span v-if="group.memberCount > group.members.length">
          {{ tr('· 预览前') }} {{ group.members.length }} {{ tr('张，打开后包含整组') }}
          {{ group.memberCount }} {{ tr('张') }}</span
        >
      </p>
    </article>
    <EmptyState
      v-if="!result?.items.length && !error"
      icon="clock"
      :title="loading ? tr('正在整理拍摄组…') : tr('没有找到连续拍摄的照片')"
      :description="tr('可调整间隔或来源；缺少拍摄时间或机型信息的照片不会被推测入组。')"
    />
    <button v-if="result?.nextCursor" class="btn" :disabled="loading || !!opening" @click="load(true)">
      {{ tr('载入更多拍摄组') }}
    </button>
  </div>
</template>
<style scoped>
.burst-preview {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(100px, 1fr));
  gap: 12px;
  padding: 18px 20px 0;
}
.burst-preview > div {
  min-width: 0;
  max-width: 220px;
}
.burst-preview :deep(.thumbnail) {
  height: 120px;
  width: 100%;
  border-radius: var(--r-sm);
}
.burst-preview span {
  display: block;
  font-size: 12px;
  margin-top: 6px;
}
.burst-folder {
  padding: 0 20px 14px;
  overflow-wrap: anywhere;
}
</style>
