<script setup lang="ts">
import CustomSelect from '../components/CustomSelect.vue';
import { computed, onUnmounted, shallowRef, watch } from 'vue';
import { useRoute } from 'vue-router';
import { api, errorMessage } from '../services/api';
import { useApp } from '../stores/app';
import type { StoryExportPreview, StoryItem, TripStory } from '../services/contracts';
import Icon from '../components/Icon.vue';
import MediaThumbnail from '../components/MediaThumbnail.vue';
import EmptyState from '../components/EmptyState.vue';

const route = useRoute(),
  app = useApp();
const tripId = computed(() => String(route.params.id ?? ''));
const range = shallowRef<'all' | 'keep'>('all');
const story = shallowRef<TripStory | null>(null),
  items = shallowRef<StoryItem[]>([]);
const loading = shallowRef(false),
  loadError = shallowRef('');
const chapters = computed(() => {
  const groups: { date: string | null; items: StoryItem[] }[] = [];
  for (const item of items.value) {
    let last = groups.at(-1);
    if (!last || last.date !== item.date) {
      last = { date: item.date, items: [] };
      groups.push(last);
    }
    last.items.push(item);
  }
  return groups;
});
const cover = computed(
  () => items.value.find((item) => item.assetId === story.value?.trip.coverAssetId) ?? items.value[0],
);
let generation = 0;
async function load(reset: boolean) {
  if (!reset && (loading.value || story.value?.nextOffset == null)) return;
  if (reset) {
    generation++;
    story.value = null;
    items.value = [];
  }
  const current = generation;
  loading.value = true;
  loadError.value = '';
  try {
    const result = await api('get_trip_story', {
      tripId: tripId.value,
      range: range.value,
      offset: reset ? 0 : story.value!.nextOffset!,
      pageSize: 100,
      epoch: reset ? null : story.value!.epoch,
    });
    if (current !== generation) return;
    story.value = result;
    items.value = reset ? result.items : [...items.value, ...result.items];
  } catch (e) {
    if (current === generation) loadError.value = errorMessage(e);
  } finally {
    if (current === generation) loading.value = false;
  }
}
const exportOpen = shallowRef(false),
  title = shallowRef(''),
  description = shallowRef('');
const includeNotes = shallowRef(false),
  destination = shallowRef('');
const plan = shallowRef<StoryExportPreview | null>(null),
  exportBusy = shallowRef(false);
const exportError = shallowRef(''),
  jobId = shallowRef('');
let exportGeneration = 0,
  activity = 0,
  mutationId = '';
function openExport() {
  title.value = story.value?.trip.name ?? '';
  description.value = story.value?.trip.description ?? '';
  includeNotes.value = false;
  destination.value = '';
  plan.value = null;
  jobId.value = '';
  exportError.value = '';
  exportOpen.value = true;
}
watch([title, description, includeNotes, destination, range, tripId, exportOpen], () => {
  exportGeneration++;
  plan.value = null;
  exportError.value = '';
  activity++;
  exportBusy.value = false;
});
watch(
  [tripId, range],
  () => {
    exportOpen.value = false;
    void load(true);
  },
  { immediate: true },
);
onUnmounted(() => {
  generation++;
  exportGeneration++;
  activity++;
});
async function pick() {
  const current = exportGeneration;
  exportBusy.value = true;
  const turn = ++activity;
  try {
    const selected = await api('pick_directory', { purpose: 'export' });
    if (current === exportGeneration && selected.path) destination.value = selected.path;
  } catch (e) {
    if (current === exportGeneration) exportError.value = errorMessage(e);
  } finally {
    if (turn === activity) exportBusy.value = false;
  }
}
async function preview() {
  const current = exportGeneration,
    turn = ++activity;
  exportBusy.value = true;
  exportError.value = '';
  plan.value = null;
  try {
    const result = await api('preview_story_export', {
      tripId: tripId.value,
      range: range.value,
      title: title.value.trim(),
      description: description.value,
      includeNotes: includeNotes.value,
      destinationPath: destination.value,
    });
    if (current === exportGeneration) {
      plan.value = result;
      mutationId = crypto.randomUUID();
      jobId.value = '';
    }
  } catch (e) {
    if (current === exportGeneration) exportError.value = errorMessage(e);
  } finally {
    if (turn === activity) exportBusy.value = false;
  }
}
async function start() {
  if (!plan.value || exportBusy.value) return;
  const current = exportGeneration,
    turn = ++activity;
  exportBusy.value = true;
  exportError.value = '';
  try {
    const result = await api('start_story_export', {
      planToken: plan.value.planToken,
      clientMutationId: mutationId,
    });
    if (current === exportGeneration) {
      jobId.value = result.jobId;
      plan.value = null;
    }
    await app.refresh();
  } catch (e) {
    if (current === exportGeneration) exportError.value = errorMessage(e);
  } finally {
    if (turn === activity) exportBusy.value = false;
  }
}
</script>
<template>
  <section class="standard-page story-page">
    <div class="story-toolbar">
      <RouterLink :to="`/trips/${tripId}`" class="btn"
        ><Icon name="back" :size="16" />返回旅行图库</RouterLink
      >
      <div class="row">
        <label
          >故事范围
          <CustomSelect
            v-model="range"
            aria-label="故事范围"
            :options="[
              { value: 'all', label: '全部旅行内容' },
              { value: 'keep', label: '仅保留项' },
            ]"
        /></label>
        <button class="btn primary" :disabled="!story?.total || loading" @click="openExport">
          <Icon name="export" :size="16" />导出离线故事
        </button>
      </div>
    </div>
    <div v-if="loadError" class="notice warning" role="alert">
      {{ loadError }} <button class="text-btn" @click="load(true)">刷新故事</button>
    </div>
    <p v-if="loading && !story" role="status">正在整理旅途中的回忆…</p>
    <template v-if="story">
      <header class="story-header">
        <div class="eyebrow">A JOURNEY TO REMEMBER</div>
        <h1>{{ story.trip.name }}</h1>
        <p class="story-description">{{ story.trip.description || '把旅途中的瞬间，串成自己的故事。' }}</p>
        <p class="muted">
          {{ story.total }} 项 · {{ range === 'keep' ? '仅保留项' : '全部旅行内容' }} · 按原始拍摄日期排列
        </p>
        <MediaThumbnail
          v-if="cover"
          :asset-id="cover.assetId"
          :alt="cover.filename"
          variant="screen"
          fit="contain"
        />
      </header>
      <p class="muted small">
        显示已启用来源中的旅行成员，包含关联媒体的各个文件。没有可信拍摄日期的项目放在末章。
      </p>
      <section v-for="chapter in chapters" :key="chapter.date ?? 'unknown'" class="story-chapter">
        <h2>{{ chapter.date ?? '未知拍摄日期' }}</h2>
        <div class="story-frames">
          <figure v-for="item in chapter.items" :key="item.assetId" class="story-frame">
            <MediaThumbnail :asset-id="item.assetId" :alt="item.filename" variant="medium" fit="contain" />
            <figcaption>
              <strong>{{ item.filename }}</strong
              ><span v-if="item.mediaKind === 'video'" class="state-pill">视频静帧</span
              ><small v-if="item.unknownTimezone">时区未知，保留原始日期</small>
              <p v-if="item.note">{{ item.note }}</p>
            </figcaption>
          </figure>
        </div>
      </section>
      <EmptyState
        v-if="!story.total"
        icon="images"
        title="这里还没有故事内容"
        description="可在旅行图库中加入媒体，或切换到全部旅行内容。"
      />
      <div v-if="story.nextOffset !== null" class="story-more">
        <p class="muted small">已载入 {{ items.length }} / {{ story.total }} 项</p>
        <button class="btn" :disabled="loading" @click="load(false)">
          {{ loading ? '正在载入…' : '继续阅读' }}
        </button>
      </div>
    </template>
    <ElDialog v-model="exportOpen" title="导出离线旅行故事" width="600px" :close-on-click-modal="false">
      <form class="form-stack" @submit.prevent="preview">
        <p class="notice">
          生成 HTML、JPEG 预览图和 SHA-256 校验清单。视频仅导出静帧；原文件未包含。预览图重新编码，不写入 GPS
          字段或源目录路径。分享时请保留整个故事文件夹。
        </p>
        <label
          >故事标题<input v-model="title" required maxlength="100" :disabled="exportBusy || !!jobId"
        /></label>
        <label
          >故事简介<textarea
            v-model="description"
            rows="3"
            maxlength="4000"
            :disabled="exportBusy || !!jobId"
          />
        </label>
        <label class="checkbox-row"
          ><input
            v-model="includeNotes"
            type="checkbox"
            :disabled="exportBusy || !!jobId"
          />包含整理备注（可能含私人文字）</label
        >
        <p class="muted small">
          范围：{{
            range === 'keep' ? '仅保留项' : '全部旅行内容'
          }}。预演会固定当时的成员、日期、标题和备注；最多 5000 项。
        </p>
        <label
          >导出到
          <div class="input-pair">
            <input
              :value="destination"
              readonly
              placeholder="选择目标文件夹"
              aria-label="故事导出目录"
            /><button
              type="button"
              class="btn"
              aria-label="选择故事导出文件夹"
              :disabled="exportBusy || !!jobId"
              @click="pick"
            >
              选择文件夹
            </button>
          </div></label
        >
        <p v-if="exportError" class="notice warning" role="alert">{{ exportError }}</p>
        <div v-if="plan" class="notice story-export-preview">
          <p>{{ plan.itemCount }} 项 · {{ plan.offlineCount }} 项当前离线或缺失</p>
          <p>
            新建文件夹：<code>{{ plan.folder }}</code>
          </p>
          <p>预演 15 分钟有效。离线或无法解码的项目会保留占位并记入失败明细。</p>
        </div>
        <p v-if="jobId" class="notice" role="status">
          故事任务已加入后台。可在任务中心暂停、取消并查看 HTML 与校验报告。
        </p>
        <div class="dialog-actions">
          <button v-if="!jobId" class="btn" :disabled="exportBusy || !destination || !title.trim()">
            {{ exportBusy ? '处理中…' : '预演故事导出' }}
          </button>
          <button v-if="plan" type="button" class="btn primary" :disabled="exportBusy" @click="start">
            开始导出 {{ plan.itemCount }} 项
          </button>
          <RouterLink v-if="jobId" to="/jobs" class="btn primary" @click="exportOpen = false"
            >查看任务与报告</RouterLink
          >
        </div>
      </form>
    </ElDialog>
  </section>
</template>
<style scoped>
.story-toolbar {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
  align-items: center;
}
.story-toolbar label {
  display: flex;
  gap: 8px;
  align-items: center;
  font-size: 13px;
  white-space: nowrap;
}
.story-toolbar .custom-select {
  width: auto;
  min-width: 160px;
}
.story-header {
  max-width: 960px;
  margin: 56px auto 36px;
  text-align: center;
}
.story-header h1 {
  font-family: var(--font-display);
  font-size: clamp(32px, 5vw, 60px);
  margin: 14px 0;
  overflow-wrap: anywhere;
}
.story-description {
  white-space: pre-wrap;
  line-height: 1.8;
}
.story-header :deep(.thumbnail) {
  height: min(52vw, 560px);
  border-radius: var(--r-xl);
  margin-top: 32px;
  background: var(--surface-raised);
}
.story-chapter {
  margin: 48px auto;
  max-width: 1100px;
}
.story-chapter h2 {
  border-top: var(--bw) solid var(--line);
  padding-top: 24px;
  font-size: 22px;
}
.story-frames {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 300px), 1fr));
  gap: 28px;
}
.story-frame {
  margin: 0;
  min-width: 0;
  padding: 12px;
  border: var(--bw) solid var(--line);
  border-radius: var(--r-lg);
  background: var(--surface);
  box-shadow: var(--shadow);
}
.story-frame :deep(.thumbnail) {
  height: 300px;
  border-radius: var(--r-lg);
  background: var(--surface-raised);
}
.story-frame figcaption {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding: 12px 0;
  align-items: center;
  overflow-wrap: anywhere;
}
.story-frame figcaption strong {
  font-size: 14px;
}
.story-frame figcaption small {
  color: var(--muted);
  font-size: 12px;
}
.story-frame figcaption p {
  width: 100%;
  white-space: pre-wrap;
  margin: 0;
  line-height: 1.7;
}
.story-more {
  text-align: center;
  padding: 16px 0 48px;
}
.story-export-preview code {
  overflow-wrap: anywhere;
}
</style>
