<script setup lang="ts">
import { computed, onMounted, onUnmounted, shallowRef, watch, useTemplateRef, useId } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { isDemo } from '../services/api';
import Icon from './Icon.vue';
import { isMediaKeyboardBlocked } from '../composables/keyboard';
const app = useApp(),
  lib = useLibrary(),
  route = useRoute(),
  router = useRouter(),
  keyword = shallowRef(lib.filter.keyword ?? '');
const searchErrorId = `${useId()}-search-error`;
const searchError = computed(() =>
  new TextEncoder().encode(keyword.value).byteLength > 2000 ? '搜索内容过长，请减少文字或表情。' : '',
);
const labels: Record<string, string> = {
  library: '全部媒体',
  filter: '筛选结果',
  sources: '来源管理',
  duplicates: '重复与相似',
  tags: '标签管理',
  jobs: '任务中心',
  settings: '设置',
  missing: '文件状态',
  trips: '旅行相册',
  compare: '多图对比',
  timeline: '时间轴',
  map: '足迹地图',
  ai: '本地 AI',
  welcome: '欢迎使用',
};
const title = computed(() => labels[route.path.split('/')[1] ?? 'library'] ?? '资料库');
const searchInput = useTemplateRef<HTMLInputElement>('searchInput');
function focusSearch(event: KeyboardEvent) {
  if (
    event.key === '/' &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !isMediaKeyboardBlocked(event)
  ) {
    event.preventDefault();
    searchInput.value?.focus();
  }
}
onMounted(() => document.addEventListener('keydown', focusSearch));
onUnmounted(() => document.removeEventListener('keydown', focusSearch));
let timeout: ReturnType<typeof setTimeout> | undefined;
function search() {
  const value = keyword.value;
  clearTimeout(timeout);
  if (searchError.value) return;
  timeout = setTimeout(() => {
    lib.setFilter({ ...lib.filter, keyword: value || undefined });
    void router.push('/library');
    void lib.search();
  }, 200);
}
watch(
  () => lib.filter.keyword,
  (value) => {
    clearTimeout(timeout);
    if (keyword.value !== (value ?? '')) keyword.value = value ?? '';
  },
);
onUnmounted(() => clearTimeout(timeout));
</script>
<template>
  <header class="topbar">
    <div class="breadcrumb">
      <button
        class="icon-btn collapse-btn"
        aria-label="收起或展开导航"
        @click="app.navCollapsed = !app.navCollapsed"
      >
        <Icon name="collapse" /></button
      ><span>我的资料库</span><Icon name="right" :size="13" /><strong>{{ title }}</strong>
    </div>
    <label class="global-search" :class="{ 'has-error': searchError }"
      ><Icon name="search" :size="17" /><input
        ref="searchInput"
        v-model="keyword"
        @input="search"
        placeholder="搜索文件名、文件夹或标签…"
        aria-label="搜索媒体"
        :aria-invalid="searchError ? true : undefined"
        :aria-describedby="searchError ? searchErrorId : undefined"
      /><kbd>/</kbd
      ><small v-if="searchError" :id="searchErrorId" class="search-field-error" role="alert">{{
        searchError
      }}</small></label
    >
    <div class="topbar-actions">
      <span v-if="isDemo" class="demo-pill">演示模式</span
      ><button class="icon-btn" aria-label="快捷键帮助" @click="app.helpOpen = true">
        <Icon name="help" /></button
      ><button class="btn primary small-btn" @click="app.importOpen = true">
        <Icon name="plus" :size="16" />添加照片目录
      </button>
    </div>
  </header>
</template>
<style scoped>
.global-search {
  position: relative;
}
.global-search.has-error {
  border-color: var(--danger);
  box-shadow: 0 0 0 1px var(--danger);
}
.search-field-error {
  position: absolute;
  top: calc(100% + 2px);
  left: 0;
  color: var(--danger);
  font-size: 11px;
  line-height: 14px;
  white-space: nowrap;
}
</style>
