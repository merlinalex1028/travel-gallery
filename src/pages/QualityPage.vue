<script setup lang="ts">
import { tr } from '../i18n';

import CustomSelect from '../components/CustomSelect.vue';
import { isDemo } from '../services/api';
import { useQuality } from '../composables/useQuality';
import AiModelPanel from '../components/local-ai/AiModelPanel.vue';
import QualitySummary from '../components/quality/QualitySummary.vue';
import QualityPreview from '../components/quality/QualityPreview.vue';
import EmptyState from '../components/EmptyState.vue';
import Icon from '../components/Icon.vue';
const {
  app,
  source,
  trip,
  range,
  onlyHints,
  includeDismissed,
  offset,
  status,
  modelStatus,
  results,
  statusError,
  error,
  message,
  busy,
  loading,
  selected,
  active,
  modelBusy,
  canStart,
  refresh,
  toggleEyes,
  toggleDetail,
  importModel,
  removeModel,
  start,
  dismiss,
  select,
  compare,
  view,
} = useQuality();
</script>
<template>
  <section class="standard-page quality-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">TAKE A CLOSER LOOK</div>
        <h1>{{ tr('照片检查') }}</h1>
        <p>{{ tr('找出值得放大检查的瞬间，由你决定留下哪张。') }}</p>
      </div>
      <button class="btn" @click="refresh"><Icon name="refresh" :size="15" />{{ tr('刷新') }}</button>
    </div>
    <p v-if="statusError || error" class="notice warning" role="alert">{{ statusError || error }}</p>
    <p v-if="message" class="notice" role="status">
      {{ message }} <RouterLink to="/jobs">{{ tr('查看任务') }}</RouterLink>
    </p>
    <AiModelPanel
      kind="eyes"
      :status="modelStatus"
      :action-busy="busy"
      :active="!!active.length"
      :model-busy="modelBusy"
      @import="importModel"
      @toggle="toggleEyes"
      @remove="removeModel"
    />
    <section class="settings-card detail-setting">
      <div>
        <h2>{{ tr('相近照片的细节对照') }}</h2>
        <p>{{ tr('默认关闭，无需模型。对固定尺寸预览测量边缘细节，供同一场景的照片比较。') }}</p>
        <p class="muted small">
          {{ tr('纹理、噪点和构图都会影响数值；不同场景之间不能据此判断好坏，RAW 使用内嵌预览。') }}
        </p>
      </div>
      <button class="btn" :disabled="isDemo || !status || busy" @click="toggleDetail">
        {{ status?.detailEnabled ? tr('关闭细节测量') : tr('启用细节测量') }}
      </button>
    </section>
    <section class="settings-card scope-card">
      <div class="quality-controls">
        <label
          >{{ tr('来源范围')
          }}<CustomSelect
            v-model="source"
            :aria-label="tr('检查来源范围')"
            :options="[
              { value: '', label: tr('全部已启用来源') },
              ...app.sources
                .filter((s) => s.isEnabled)
                .map((item) => ({ value: item.id, label: item.displayName })),
            ]"
        /></label>
        <label
          >{{ tr('旅行范围')
          }}<CustomSelect
            v-model="trip"
            :aria-label="tr('检查旅行范围')"
            :options="[
              { value: '', label: tr('全部已启用来源') },
              ...app.trips.map((item) => ({ value: item.id, label: item.name })),
            ]" /></label
        ><label
          >{{ tr('整理状态')
          }}<CustomSelect
            v-model="range"
            :aria-label="tr('检查整理范围')"
            :options="[
              { value: 'all', label: tr('全部照片') },
              { value: 'keep', label: tr('仅保留照片') },
            ]"
        /></label>
      </div>
      <p class="scope-count">
        <strong>{{ status?.total ?? '—' }}</strong> {{ tr('张照片 · 闭眼分析') }} {{ status?.eyesCount ?? 0 }}
        {{ tr('项 · 细节测量') }} {{ status?.detailCount ?? 0 }} {{ tr('项') }}
      </p>
      <button class="btn primary" :disabled="isDemo || !canStart" @click="start">
        {{ busy ? tr('正在提交…') : tr('检查当前范围') }}
      </button>
      <p class="muted small">
        {{ tr('仅运行已启用的项目，复用未变化的记录。单次最多 100000 张，开始后固定范围；视频不参与。') }}
      </p>
      <p v-if="active.length" class="notice">
        {{ tr('已有本地分析任务。')
        }}<RouterLink to="/jobs">{{ tr('查看进度、暂停、继续或取消') }}</RouterLink
        >{{ tr('。关闭对应分析可停止任务。') }}
      </p>
    </section>
    <p class="notice">
      {{
        tr(
          '提示用于人工复核，不会自动淘汰或改变评分。远景、眼镜和遮挡可能误报或漏报；未检出人脸或未触发提示不保证所有人睁眼。',
        )
      }}
    </p>
    <div class="row results-heading">
      <h2>{{ tr('检查记录') }}</h2>
      <button class="btn" :disabled="selected.size < 2 || selected.size > 4 || loading" @click="compare">
        {{ tr('对比所选') }} {{ selected.size }} {{ tr('张') }}
      </button>
    </div>
    <div class="quality-controls result-controls">
      <label><input v-model="onlyHints" type="checkbox" />{{ tr('仅看眼睛检查提示') }}</label
      ><label><input v-model="includeDismissed" type="checkbox" />{{ tr('包括已忽略记录') }}</label
      ><span class="muted small">{{ tr('每页 60 项，可选 2–4 张进入同步对比。') }}</span>
    </div>
    <p v-if="loading" role="status">{{ tr('正在读取检查记录…') }}</p>
    <div v-else-if="results?.items.length" class="quality-grid">
      <article v-for="item in results.items" :key="item.asset.id" class="quality-card">
        <button
          class="preview-button"
          :aria-label="tr('放大检查 {0}', { 0: item.asset.filename })"
          :disabled="!item.analysis.observations.length"
          @click="view(item.asset.id)"
        >
          <QualityPreview
            :asset-id="item.asset.id"
            :filename="item.asset.filename"
            :analysis="item.analysis"
          />
        </button>
        <div class="row card-heading">
          <label
            ><input
              type="checkbox"
              :checked="selected.has(item.asset.id)"
              :disabled="
                !item.analysis.observations.length || (!selected.has(item.asset.id) && selected.size >= 4)
              "
              :aria-label="tr('选择对比 {0}', { 0: item.asset.filename })"
              @change="select(item.asset.id)"
            /><strong>{{ item.asset.filename }}</strong></label
          >
        </div>
        <QualitySummary :analysis="item.analysis" :busy="busy" @dismiss="dismiss(item.asset.id, $event)" />
      </article>
    </div>
    <EmptyState
      v-else-if="results"
      icon="check"
      :title="tr('当前范围没有匹配的检查记录')"
      :description="tr('可运行检查、查看全部记录，或调整旅行与整理范围。')"
    />
    <div v-if="results && (results.total > 60 || offset)" class="quality-pagination">
      <button class="btn" :disabled="!offset || loading" @click="offset = Math.max(0, offset - 60)">
        {{ tr('上一页') }}</button
      ><span>{{ offset + 1 }}–{{ Math.min(offset + 60, results.total) }} / {{ results.total }}</span
      ><button class="btn" :disabled="offset + 60 >= results.total || loading" @click="offset += 60">
        {{ tr('下一页') }}
      </button>
    </div>
  </section>
</template>
<style scoped>
.quality-page h2 {
  font-size: 20px;
  margin: 0 0 10px;
}
.quality-page .settings-card {
  margin-bottom: 24px;
}
.detail-setting {
  display: flex;
  gap: 24px;
  align-items: center;
  justify-content: space-between;
}
.detail-setting button {
  flex-shrink: 0;
}
.detail-setting p {
  line-height: 1.8;
  margin: 8px 0;
}
.quality-controls {
  display: flex;
  gap: 20px;
  flex-wrap: wrap;
  align-items: center;
}
.quality-controls label {
  display: flex;
  gap: 10px;
  align-items: center;
}
.quality-controls .custom-select {
  min-width: 150px;
  width: auto;
  max-width: 340px;
}
.quality-controls input[type='checkbox'],
.card-heading input[type='checkbox'] {
  width: 17px;
  accent-color: var(--accent);
}
.scope-count {
  margin: 24px 0 16px;
  color: var(--muted);
}
.scope-count strong {
  font-size: 26px;
  color: var(--text);
}
.results-heading {
  margin: 32px 0 16px;
}
.result-controls {
  margin-bottom: 24px;
}
.quality-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(245px, 1fr));
  gap: 26px;
}
.quality-card {
  min-width: 0;
  padding: 14px;
  border: var(--bw) solid var(--line);
  border-radius: var(--r-lg);
  background: var(--surface);
  box-shadow: var(--shadow);
}
.preview-button {
  display: block;
  border: 0;
  padding: 0;
  width: 100%;
  border-radius: var(--r-sm);
  cursor: zoom-in;
  background: transparent;
}
.card-heading {
  margin-top: 12px;
}
.card-heading label {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  overflow-wrap: anywhere;
}
.card-heading strong {
  font-size: 13px;
}
.quality-pagination {
  display: flex;
  gap: 20px;
  align-items: center;
  justify-content: center;
  margin: 30px 0;
}
@media (max-width: 1100px) {
  .detail-setting {
    flex-wrap: wrap;
  }
}
@media (max-width: 780px) {
  .quality-controls label {
    flex-wrap: wrap;
    max-width: 100%;
  }
  .quality-controls .custom-select {
    min-width: 0;
    max-width: 100%;
  }
}
</style>
