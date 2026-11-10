<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue';
import type { Commands, ExportReportItem } from '../services/contracts';

const open = defineModel<boolean>({ required: true });
const props = defineProps<{ report: Commands['get_export_report']['result'] | null }>();
const page = shallowRef(1);
const stopped = computed(
  () => !!props.report && ['completed', 'failed', 'cancelled'].includes(props.report.status),
);
const pending = (items: ExportReportItem[]) =>
  items.filter((item) => ['queued', 'copying'].includes(item.state)).length;
const pendingMedia = computed(() => pending(props.report?.items ?? []));
const pendingSidecars = computed(() => pending(props.report?.sidecars ?? []));
const rows = computed(() => [
  ...(props.report?.items ?? []).map((item) => ({ item, sidecar: false })),
  ...(props.report?.sidecars ?? []).map((item) => ({ item, sidecar: true })),
]);
const pageCount = computed(() => Math.max(1, Math.ceil(rows.value.length / 100)));
const visibleRows = computed(() => rows.value.slice((page.value - 1) * 100, page.value * 100));
watch(
  () => [props.report?.jobId, open.value],
  () => {
    page.value = 1;
  },
);
const states: Record<string, string> = {
  queued: '待复制',
  copying: '复制中',
  copied: '复制完成',
  skipped: '已跳过',
  failed: '失败',
};
const errors: Record<string, string> = {
  EXPORT_CONFLICT: '目标同名或无法安全发布',
  ASSET_NOT_FOUND: '源文件缺失',
  SOURCE_OFFLINE: '来源离线',
  DB_CONFLICT: '文件已变化，请重新预演',
  EXPORT_NO_SPACE: '目标空间不足或无法写入',
  PERMISSION_DENIED: '授权已失效',
  CANCELLED: '用户取消，未生成完整副本',
  MEDIA_EXPORT_FAILED: '媒体复制未完成，XMP 未写入',
};
function stateLabel(item: ExportReportItem) {
  if (stopped.value && item.state === 'queued')
    return props.report?.status === 'cancelled' ? '未开始（任务已取消）' : '未开始（任务已结束）';
  if (stopped.value && item.state === 'copying') return '未完成（任务已结束）';
  if (item.state === 'failed' && item.errorCode === 'CANCELLED') return '复制中止';
  return states[item.state] ?? item.state;
}
function detailLabel(item: ExportReportItem) {
  if (stopped.value && ['queued', 'copying'].includes(item.state)) return '请重新预演后复制';
  return item.errorCode
    ? (errors[item.errorCode] ?? '操作失败，请检查任务明细')
    : item.sha256
      ? '已校验'
      : '等待处理';
}
</script>
<template>
  <ElDialog v-model="open" :title="report?.story ? '故事导出报告' : '复制导出报告'" width="760px">
    <template v-if="report">
      <div class="stat-grid report-stats">
        <div>
          <strong>{{ report.copiedCount }}</strong
          ><span>{{ report.story ? 'JPEG 预览完成' : '复制完成' }}</span>
        </div>
        <div>
          <strong>{{ report.verifiedCount }}</strong
          ><span>SHA-256 校验通过</span>
        </div>
        <div>
          <strong>{{ report.skippedCount }}</strong
          ><span>已跳过</span>
        </div>
        <div>
          <strong>{{ report.failedCount }}</strong
          ><span>失败或中止</span>
        </div>
        <div aria-label="未完成媒体">
          <strong>{{ pendingMedia }}</strong
          ><span>未完成媒体</span>
        </div>
      </div>
      <div v-if="report.story" class="notice">
        <p>
          故事文件夹：<code>{{ report.story.folder }}</code>
        </p>
        <p>
          {{
            report.story.result?.html
              ? 'index.html 已写入并通过 SHA-256 校验；用浏览器打开即可离线阅读。'
              : 'HTML 尚未完成；已生成的预览图保留在目标目录。'
          }}
        </p>
        <p>
          {{
            report.story.result?.manifest ? 'manifest.json 校验清单已完成。' : '校验清单尚未完成。'
          }}视频以静帧呈现，原文件未包含。整理备注{{ report.story.includeNotes ? '已包含' : '未包含' }}。
        </p>
      </div>
      <p v-if="report.sidecars.length" class="notice" :class="{ warning: report.xmpFailedCount }">
        XMP：{{ report.xmpCopiedCount }} 完成 · {{ report.xmpSkippedCount }} 跳过 ·
        {{ report.xmpFailedCount }} 失败或中止 · {{ pendingSidecars }} 未完成。XMP 失败时已复制的媒体仍保留。
      </p>
      <div class="notice" role="status">
        <template v-if="report.status === 'cancelled'">任务已取消。</template>
        <template v-else-if="stopped">任务已结束。</template>
        <template v-else>任务仍在进行；再次打开报告可查看最新结果。</template>
        已完成文件保留在目标目录。
        <template
          v-if="stopped && (pendingMedia || pendingSidecars || report.failedCount || report.xmpFailedCount)"
          >未完成文件不会自动继续；请重新预演后发起新的复制任务。</template
        >
      </div>
      <p v-if="report.manifestPath" class="small path-text">CSV 结果清单：{{ report.manifestPath }}</p>
      <p v-else-if="stopped && !report.story" class="small muted">
        本次未生成 CSV 清单；可通过逐项报告核对已完成文件。
      </p>
      <div class="report-table">
        <div v-for="row in visibleRows" :key="`${row.sidecar ? 'xmp' : 'media'}-${row.item.assetId}`">
          <span class="path-text">{{ row.sidecar ? 'XMP · ' : '' }}{{ row.item.destination }}</span>
          <strong>{{ stateLabel(row.item) }}</strong
          ><small>{{ detailLabel(row.item) }}</small>
        </div>
      </div>
      <nav v-if="rows.length > 100" class="row report-pagination" aria-label="导出报告分页">
        <button class="btn small-btn" :disabled="page === 1" @click="page--">上一页</button>
        <span>第 {{ page }} / {{ pageCount }} 页 · 共 {{ rows.length }} 项（含 XMP）</span>
        <button class="btn small-btn" :disabled="page === pageCount" @click="page++">下一页</button>
      </nav>
    </template>
  </ElDialog>
</template>
<style scoped>
.report-stats {
  flex-wrap: wrap;
  gap: 16px;
}
.report-stats > div {
  min-width: 100px;
}
.report-pagination {
  justify-content: space-between;
  margin-top: 16px;
  flex-wrap: wrap;
}
</style>
