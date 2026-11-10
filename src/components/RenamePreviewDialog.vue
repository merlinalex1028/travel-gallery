<script setup lang="ts">
import { onUnmounted, shallowRef, watch, computed } from 'vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api, errorMessage } from '../services/api';
import type { NamingRule, RenamePreview, SelectionTarget } from '../services/contracts';
import NamingFields from './NamingFields.vue';
const app = useApp(),
  lib = useLibrary();
const rule = shallowRef<NamingRule>({ pattern: '{date}_{seq}', start: 1, padding: 4 }),
  target = shallowRef<SelectionTarget>({ kind: 'explicit', assetIds: [] }),
  count = shallowRef(0),
  result = shallowRef<RenamePreview | null>(null),
  busy = shallowRef(false),
  offset = shallowRef(0);
const error = shallowRef('');
const states = {
  ready: '可用',
  unchanged: '名称不变',
  conflict: '名称冲突',
  invalid: '名称无效',
  unavailable: '来源不可用',
};
const running = computed(
  () => !!result.value && ['queued', 'running', 'paused'].includes(result.value.job.status),
);
let generation = 0,
  timer: ReturnType<typeof setInterval> | undefined,
  polling = false;
function stop() {
  if (timer) clearInterval(timer);
  timer = undefined;
}
async function read(page = offset.value) {
  if (!app.renameJobId || polling) return;
  const seq = generation,
    id = app.renameJobId;
  polling = true;
  const value = await app.perform(async () => {
    try {
      return await api('get_rename_preview', { jobId: id, offset: page, pageSize: 100 });
    } catch (e) {
      if (seq === generation && app.renameOpen) {
        error.value = errorMessage(e);
        stop();
      }
      throw e;
    }
  });
  polling = false;
  if (value && seq === generation && app.renameOpen && id === app.renameJobId) {
    result.value = value;
    rule.value = { ...value.rule };
    offset.value = page;
    if (!['queued', 'running', 'paused'].includes(value.job.status)) stop();
  }
}
watch(
  () => app.renameOpen,
  (open) => {
    generation++;
    stop();
    result.value = null;
    offset.value = 0;
    busy.value = false;
    error.value = '';
    if (!open) return;
    if (!app.renameJobId) {
      target.value = JSON.parse(JSON.stringify(lib.target)) as SelectionTarget;
      count.value = lib.count;
    } else {
      void read();
      timer = setInterval(() => void read(), 800);
    }
  },
);
async function start() {
  const seq = ++generation;
  busy.value = true;
  result.value = null;
  offset.value = 0;
  const value = await app.perform(() =>
    api('preview_rename', { target: target.value, rule: { ...rule.value } }),
  );
  if (seq === generation && app.renameOpen) {
    busy.value = false;
    if (value) {
      app.renameJobId = value.jobId;
      await read();
      stop();
      if (running.value || !result.value) timer = setInterval(() => void read(), 800);
      void app.perform(app.refresh);
    }
  }
}
async function cancel() {
  if (app.renameJobId) await app.perform(() => api('cancel_job', { jobId: app.renameJobId }));
  await read();
}
function reset() {
  generation++;
  stop();
  app.renameJobId = '';
  result.value = null;
  offset.value = 0;
  target.value = JSON.parse(JSON.stringify(lib.target)) as SelectionTarget;
  count.value = lib.count;
}
function exportCopies() {
  if (!result.value?.summary) return;
  app.exportDraft = {
    target: result.value.target,
    count: result.value.summary.total,
    naming: { ...result.value.rule },
  };
  app.renameOpen = false;
  app.exportOpen = true;
}
onUnmounted(() => {
  generation++;
  stop();
});
</script>
<template>
  <ElDialog
    v-model="app.renameOpen"
    title="批量重命名预演"
    width="880px"
    class="rename-dialog"
    :close-on-click-modal="false"
  >
    <p class="dialog-lead">只预演名称，不改写或移动源文件。可以将规则用于复制导出，再确认目标路径。</p>
    <p v-if="error" class="warning-text" role="alert">{{ error }}</p>
    <template v-if="!app.renameJobId"
      ><p>本次选择已冻结：{{ count.toLocaleString() }} 个媒体。</p>
      <NamingFields v-model="rule" :disabled="busy"
    /></template>
    <template v-else-if="result">
      <p role="status">
        {{
          running
            ? '正在分析'
            : result.job.status === 'completed'
              ? '预演已完成'
              : result.job.status === 'cancelled'
                ? '已取消'
                : '预演失败'
        }}
        · {{ result.job.progressDone }} / {{ result.job.progressTotal ?? '…' }} 项
      </p>
      <ElProgress
        v-if="running"
        :percentage="
          result.job.progressTotal
            ? Math.round((result.job.progressDone / result.job.progressTotal) * 100)
            : 0
        "
      />
      <p v-if="result.job.errorSummary" class="warning-text">{{ result.job.errorSummary }}</p>
      <template v-if="result.summary">
        <p>
          可用 {{ result.summary.ready }} · 名称不变 {{ result.summary.unchanged }} · 冲突
          {{ result.summary.conflict }} · 无效 {{ result.summary.invalid }} · 不可用
          {{ result.summary.unavailable }}
        </p>
        <div class="rename-table-wrap">
          <table class="rename-table">
            <thead>
              <tr>
                <th>来源 / 原名称</th>
                <th>拟用名称</th>
                <th>结果</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="item in result.items" :key="item.assetId">
                <td>
                  <small>{{ item.sourceName }}</small
                  ><br />{{ item.sourceRelativePath }}
                </td>
                <td>{{ item.proposedFilename ?? '—' }}<small v-if="item.dateUnknown">拍摄时间未知</small></td>
                <td :class="{ 'warning-text': ['conflict', 'invalid', 'unavailable'].includes(item.status) }">
                  {{ states[item.status] }}<small v-if="item.reason">{{ item.reason }}</small>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <div class="button-row rename-pagination">
          <button
            class="btn small-btn"
            :disabled="offset === 0 || polling"
            @click="read(Math.max(0, offset - 100))"
          >
            上一页</button
          ><span>{{ offset + 1 }}–{{ offset + result.items.length }} / {{ result.summary.total }}</span
          ><button
            class="btn small-btn"
            :disabled="result.nextOffset === null || polling"
            @click="read(result.nextOffset!)"
          >
            下一页
          </button>
        </div>
      </template>
    </template>
    <div class="notice">
      按来源内的实际文件名检查冲突，采用保守的大小写比较。分析结果保留 15
      分钟；目录发生变化后应重新预演。关闭后可在任务中心查看或取消任务。
    </div>
    <template #footer
      ><button class="btn" @click="app.renameOpen = false">关闭</button
      ><button v-if="running" class="btn" @click="cancel">取消预演</button
      ><button v-if="app.renameJobId && !running" class="btn" @click="reset">重新选择与预演</button
      ><button v-if="!app.renameJobId" class="btn primary" :disabled="busy || !count" @click="start">
        {{ busy ? '正在提交…' : '开始预演' }}</button
      ><button v-if="result?.summary" class="btn primary" @click="exportCopies">
        用于复制导出
      </button></template
    >
  </ElDialog>
</template>
