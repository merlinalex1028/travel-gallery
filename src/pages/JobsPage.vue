<script setup lang="ts">
import { tr, translatedRecord } from '../i18n';

import { computed, shallowRef } from 'vue';
import { api } from '../services/api';
import { useApp } from '../stores/app';
import { date, jobStates, jobTypes } from '../services/format';
import type { Commands } from '../services/contracts';
import Icon from '../components/Icon.vue';
import EmptyState from '../components/EmptyState.vue';
import ExportReportDialog from '../components/ExportReportDialog.vue';
const app = useApp(),
  filter = shallowRef('all'),
  errors = shallowRef<{ assetId: string | null; message: string }[]>([]),
  detailOpen = shallowRef(false),
  report = shallowRef<Commands['get_export_report']['result'] | null>(null),
  reportOpen = shallowRef(false);
const operationNames: Record<string, string> = translatedRecord({
  renamePreview: '重命名预演',
  storyExport: '故事导出',
  aiImportModel: '导入离线模型',
  aiRemoveModel: '移除本地模型和索引',
  aiIndex: '建立内容索引',
  aiCluster: 'AI 内容分组',
  eyesImportModel: '导入闭眼模型',
  eyesRemoveModel: '移除闭眼模型和记录',
  qualityAnalyze: '照片闭眼与细节检查',
  recomputeExact: '重算过期精确组',
});
const visible = computed(() =>
  app.jobs.filter(
    (j) =>
      filter.value === 'all' ||
      (filter.value === 'running'
        ? ['running', 'queued', 'paused'].includes(j.status)
        : j.status === filter.value),
  ),
);
async function control(name: 'cancel_job' | 'pause_job' | 'resume_job' | 'retry_job', jobId: string) {
  await app.perform(async () => {
    await api(name, { jobId });
    await app.refresh();
  });
}
async function details(jobId: string) {
  await app.perform(async () => {
    errors.value = await api('get_job_errors', { jobId });
    detailOpen.value = true;
  });
}
async function showReport(jobId: string) {
  await app.perform(async () => {
    report.value = await api('get_export_report', { jobId });
    reportOpen.value = true;
  });
}
</script>
<template>
  <section class="standard-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">WORKING QUIETLY IN THE BACKGROUND</div>
        <h1>
          {{ tr('任务中心') }}<span class="heading-count">{{ app.running.length }} {{ tr('进行中') }}</span>
        </h1>
        <p>{{ tr('让整理在后台继续，把注意力留给风景。') }}</p>
      </div>
      <button class="btn" @click="app.perform(app.refresh)">
        <Icon name="refresh" :size="15" />{{ tr('刷新') }}
      </button>
    </div>
    <div class="segmented wide-tabs">
      <button
        v-for="[value, label] in [
          ['all', tr('全部任务')],
          ['running', tr('进行中')],
          ['completed', tr('已完成')],
          ['failed', tr('失败')],
          ['cancelled', tr('已取消')],
        ]"
        :key="value"
        :class="{ active: filter === value }"
        @click="filter = value!"
      >
        {{ label }}
      </button>
    </div>
    <div v-if="visible.length" class="job-list">
      <article v-for="job in visible" :key="job.id" class="job-card">
        <div class="job-icon" :class="job.status">
          <Icon
            :name="
              job.type === 'SCAN'
                ? 'folder'
                : job.type === 'EXPORT'
                  ? 'export'
                  : job.type.startsWith('HASH')
                    ? 'copy'
                    : job.type === 'BACKUP'
                      ? 'disk'
                      : 'images'
            "
            :size="23"
          />
        </div>
        <div class="job-main">
          <div class="row">
            <h3>
              {{ job.operation ? (operationNames[job.operation] ?? jobTypes[job.type]) : jobTypes[job.type] }}
            </h3>
            <span class="state-pill" :class="job.status">{{
              job.cancelRequested && ['running', 'queued', 'paused'].includes(job.status)
                ? tr('正在安全取消…')
                : jobStates[job.status]
            }}</span>
          </div>
          <p>
            {{ date(job.createdAt, true) }}<span class="dot">·</span
            >{{
              job.progressTotal === null
                ? tr('已处理 {0} 项 · 总数待发现', { 0: job.progressDone })
                : tr('{0} / {1} 项', { 0: job.progressDone, 1: job.progressTotal })
            }}<span v-if="job.failureCount" class="warning-text">
              · {{ job.failureCount }} {{ tr('项失败') }}</span
            >
          </p>
          <ElProgress
            v-if="['queued', 'running', 'paused'].includes(job.status)"
            :percentage="job.progressTotal ? Math.round((job.progressDone / job.progressTotal) * 100) : 0"
            :indeterminate="job.progressTotal === null"
            :show-text="job.progressTotal !== null"
            :stroke-width="5"
          />
          <p v-if="job.errorSummary" class="warning-text">{{ job.errorSummary }}</p>
          <p
            v-if="['queued', 'running'].includes(job.status) && job.autoPauseReason === 'battery'"
            class="muted"
            role="status"
          >
            {{ tr('电池供电，后台处理已等待；接通电源或关闭电池暂停选项后自动继续。') }}
          </p>
        </div>
        <div class="job-actions">
          <template v-if="['queued', 'running'].includes(job.status) && !job.cancelRequested"
            ><button class="icon-btn" :aria-label="tr('暂停任务')" @click="control('pause_job', job.id)">
              <Icon name="pause" /></button
            ><button class="text-btn" @click="control('cancel_job', job.id)">
              {{ tr('取消') }}
            </button></template
          ><template v-else-if="job.status === 'paused'"
            ><button class="btn small-btn" @click="control('resume_job', job.id)">
              <Icon name="play" :size="14" />{{ tr('继续') }}</button
            ><button class="text-btn" @click="control('cancel_job', job.id)">
              {{ tr('取消') }}
            </button></template
          ><button
            v-else-if="
              ['failed', 'cancelled'].includes(job.status) &&
              job.type !== 'EXPORT' &&
              job.operation !== 'aiImportModel' &&
              job.operation !== 'eyesImportModel'
            "
            class="btn small-btn"
            @click="control('retry_job', job.id)"
          >
            <Icon name="refresh" :size="14" />{{ tr('重试') }}</button
          ><button v-if="job.failureCount || job.errorSummary" class="text-btn" @click="details(job.id)">
            {{ tr('失败明细') }}</button
          ><button v-if="job.type === 'EXPORT'" class="btn small-btn" @click="showReport(job.id)">
            {{ tr('导出报告') }}</button
          ><RouterLink v-if="job.operation?.startsWith('ai')" to="/ai" class="btn small-btn">{{
            tr('本地 AI')
          }}</RouterLink
          ><RouterLink
            v-if="job.operation?.startsWith('eyes') || job.operation === 'qualityAnalyze'"
            to="/quality"
            class="btn small-btn"
            >{{ tr('照片检查') }}</RouterLink
          ><button
            v-if="job.operation === 'renamePreview'"
            class="btn small-btn"
            @click="
              app.renameJobId = job.id;
              app.renameOpen = true;
            "
          >
            {{ tr('查看预演') }}
          </button>
        </div>
      </article>
    </div>
    <EmptyState
      v-else
      icon="jobs"
      :title="tr('现在，一切井然有序')"
      :description="tr('扫描、缩略图、查重、导出和备份任务会显示在这里。')"
    />
    <p class="muted small">
      {{ tr('当前显示最近 100 个任务。中断的任务会保留历史，并在重启后标为可重试。') }}
    </p>
    <ElDialog v-model="detailOpen" :title="tr('任务失败明细')" width="680px"
      ><p v-if="!errors.length">{{ tr('没有逐文件错误。任务可能被中断或取消；请查看任务卡片摘要。') }}</p>
      <div class="job-errors">
        <div v-for="(e, i) in errors" :key="i">
          <code>{{ e.assetId?.slice(0, 8) ?? tr('任务') }}</code
          ><span>{{ e.message }}</span>
        </div>
      </div></ElDialog
    ><ExportReportDialog v-model="reportOpen" :report="report" />
  </section>
</template>
