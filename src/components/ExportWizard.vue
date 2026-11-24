<script setup lang="ts">
import CustomSelect from './CustomSelect.vue';
import { computed, onUnmounted, shallowRef, watch } from 'vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api } from '../services/api';
import type {
  ExportPreview,
  SelectionTarget,
  ExportPreset,
  ExportOptions,
  NamingRule,
} from '../services/contracts';
import { bytes } from '../services/format';
import Icon from './Icon.vue';
import NamingFields from './NamingFields.vue';
const app = useApp(),
  lib = useLibrary(),
  destination = shallowRef(''),
  layout = shallowRef<'flat' | 'captureMonth' | 'sourceRelative'>('flat'),
  policy = shallowRef<'rename' | 'skip'>('rename'),
  includeLinked = shallowRef(true),
  includeXmp = shallowRef(false),
  namingEnabled = shallowRef(false),
  naming = shallowRef<NamingRule>({ pattern: '{date}_{seq}', start: 1, padding: 4 }),
  presets = shallowRef<ExportPreset[]>([]),
  presetId = shallowRef(''),
  presetName = shallowRef(''),
  preview = shallowRef<ExportPreview | null>(null),
  busy = shallowRef(false),
  frozen = shallowRef<SelectionTarget>({ kind: 'explicit', assetIds: [] }),
  selectedCount = shallowRef(0),
  jobId = shallowRef('');
const job = computed(() => app.jobs.find((j) => j.id === jobId.value));
let generation = 0;
let opening = 0;
let activity = 0;
onUnmounted(() => {
  generation++;
});
watch(
  () => app.exportOpen,
  (open) => {
    generation++;
    opening++;
    activity++;
    busy.value = false;
    if (open) {
      const draft = app.exportDraft;
      frozen.value = structuredClone(
        JSON.parse(JSON.stringify(draft?.target ?? lib.target)) as SelectionTarget,
      );
      selectedCount.value = draft?.count ?? lib.count;
      preview.value = null;
      jobId.value = '';
      destination.value = '';
      includeXmp.value = false;
      namingEnabled.value = !!draft;
      if (draft) naming.value = structuredClone(JSON.parse(JSON.stringify(draft.naming)) as NamingRule);
      app.exportDraft = null;
      presetId.value = '';
      presetName.value = '';
      const current = opening;
      void app.perform(async () => {
        const result = await api('list_export_presets', undefined);
        if (current === opening && app.exportOpen) presets.value = result;
      });
    }
  },
);
watch([destination, layout, policy, includeLinked, includeXmp, namingEnabled, naming], () => {
  generation++;
  preview.value = null;
});
const selectedPreset = computed(() => presets.value.find((p) => p.id === presetId.value));
function options(): ExportOptions {
  return {
    layout: layout.value,
    conflictPolicy: policy.value,
    includeLinked: includeLinked.value,
    includeXmp: includeXmp.value,
    naming: namingEnabled.value ? { ...naming.value } : null,
  };
}
function applyPreset() {
  const preset = selectedPreset.value;
  if (!preset) return;
  presetName.value = preset.name;
  layout.value = preset.options.layout;
  policy.value = preset.options.conflictPolicy;
  includeLinked.value = preset.options.includeLinked ?? true;
  includeXmp.value = preset.options.includeXmp ?? false;
  namingEnabled.value = !!preset.options.naming;
  if (preset.options.naming) naming.value = { ...preset.options.naming };
}
async function savePreset(replace = false) {
  const current = opening,
    prior = selectedPreset.value;
  const currentActivity = ++activity;
  busy.value = true;
  const saved = await app.perform(
    () =>
      api('save_export_preset', {
        ...(replace && prior ? { id: prior.id, expectedRevision: prior.revision } : {}),
        name: presetName.value,
        options: options(),
      }),
    '导出预设已保存',
  );
  if (saved && current === opening && app.exportOpen) {
    presets.value = [...presets.value.filter((p) => p.id !== saved.id), saved].sort((a, b) =>
      a.name.localeCompare(b.name),
    );
    presetId.value = saved.id;
  }
  if (currentActivity === activity) busy.value = false;
}
async function deletePreset() {
  const prior = selectedPreset.value;
  if (!prior) return;
  const current = opening;
  const currentActivity = ++activity;
  busy.value = true;
  const result = await app.perform(
    () => api('delete_export_preset', { id: prior.id, expectedRevision: prior.revision }),
    '预设已移除',
  );
  if (result && current === opening && app.exportOpen) {
    presets.value = presets.value.filter((p) => p.id !== prior.id);
    presetId.value = '';
    presetName.value = '';
  }
  if (currentActivity === activity) busy.value = false;
}
async function choose() {
  const seq = ++generation;
  const r = await app.perform(() => api('pick_directory', { purpose: 'export' }));
  if (r?.path && seq === generation && app.exportOpen) destination.value = r.path;
}
async function plan() {
  const seq = ++generation;
  const currentActivity = ++activity;
  busy.value = true;
  await app.perform(async () => {
    const result = await api('preview_export', {
      target: frozen.value,
      destinationPath: destination.value,
      layout: layout.value,
      conflictPolicy: policy.value,
      includeLinked: includeLinked.value,
      includeXmp: includeXmp.value,
      naming: namingEnabled.value ? { ...naming.value } : null,
    });
    if (seq === generation && app.exportOpen) preview.value = result;
  });
  if (currentActivity === activity) busy.value = false;
}
async function start() {
  if (!preview.value) return;
  const seq = ++generation;
  const currentActivity = ++activity;
  busy.value = true;
  await app.perform(async () => {
    const r = await api('start_export', {
      planToken: preview.value!.planToken,
      clientMutationId: crypto.randomUUID(),
    });
    if (seq === generation && app.exportOpen) jobId.value = r.jobId;
    await app.refresh();
  }, '导出任务已开始');
  if (currentActivity === activity) busy.value = false;
}
</script>
<template>
  <ElDialog
    v-model="app.exportOpen"
    title="复制导出精选"
    width="640px"
    class="export-dialog"
    :close-on-click-modal="false"
    ><template v-if="!jobId"
      ><p class="dialog-lead">
        本次选择已冻结：<strong>{{ selectedCount.toLocaleString() }} 个文件</strong
        >。原文件留在原处，导出不转码。
      </p>
      <div class="form-stack">
        <label
          >导出预设<CustomSelect
            v-model="presetId"
            :disabled="busy"
            aria-label="导出预设"
            @change="applyPreset"
            :options="[
              { value: '', label: '手动配置 / 新建预设' },
              ...presets.map((preset) => ({ value: preset.id, label: preset.name })),
            ]"
        /></label>
        <div class="preset-controls">
          <input
            v-model="presetName"
            :disabled="busy"
            aria-label="预设名称"
            placeholder="预设名称"
            maxlength="80"
          /><button class="btn small-btn" :disabled="busy || !presetName.trim()" @click="savePreset()">
            另存为预设</button
          ><button
            v-if="selectedPreset"
            class="text-btn"
            :disabled="busy || !presetName.trim()"
            @click="savePreset(true)"
          >
            更新预设</button
          ><button v-if="selectedPreset" class="text-btn" :disabled="busy" @click="deletePreset">
            移除预设
          </button>
        </div>
        <p class="muted small">预设保存规则；目标目录每次另选。选择含 XMP 的预设后才会启用附属文件。</p>
        <label class="check-label"
          ><input v-model="includeLinked" :disabled="busy" type="checkbox" />同时导出已确认的 Live Photo / RAW
          + JPEG 配套文件</label
        >
        <label class="check-label"
          ><input v-model="includeXmp" :disabled="busy" type="checkbox" />附带 XMP 标签、星级和备注文件</label
        >
        <p v-if="includeXmp" class="muted small">
          仅写入导出目录，保存预演时的整理记录。照片与 XMP
          以同一主文件名成对命名；任一同名冲突会让整对改名或跳过。
        </p>
        <label
          >导出目录<button :disabled="busy" class="folder-picker compact-picker" @click="choose">
            <Icon name="folder" /><span>{{ destination || '通过系统选择器授权目标目录' }}</span
            ><Icon name="right" /></button></label
        ><label
          >目录结构<CustomSelect
            v-model="layout"
            :disabled="busy"
            aria-label="目录结构"
            :options="[
              { value: 'flat', label: '全部放在目标目录' },
              { value: 'captureMonth', label: '按拍摄月份分文件夹' },
              { value: 'sourceRelative', label: '保留来源目录结构' },
            ]" /></label
        ><label
          >同名文件<CustomSelect
            v-model="policy"
            :disabled="busy"
            aria-label="同名文件"
            :options="[
              { value: 'rename', label: '自动改名，如 photo (2).jpg' },
              { value: 'skip', label: '跳过已有文件' },
            ]"
        /></label>
        <label class="check-label"
          ><input v-model="namingEnabled" :disabled="busy" type="checkbox" />自定义导出副本名称</label
        >
        <NamingFields v-if="namingEnabled" v-model="naming" :disabled="busy" />
      </div>
      <div v-if="preview" class="export-summary">
        <div>
          <span>媒体数量</span><strong>{{ preview.fileCount }}</strong>
        </div>
        <div v-if="preview.xmpCount">
          <span>XMP 附属文件</span><strong>{{ preview.xmpCount }}</strong>
        </div>
        <div>
          <span>预计大小</span><strong>{{ bytes(preview.totalBytes) }}</strong>
        </div>
        <div>
          <span>同名冲突</span><strong>{{ preview.conflictCount }}</strong>
        </div>
        <div>
          <span>原文件不可用</span><strong>{{ preview.offlineCount }}</strong>
        </div>
        <p v-for="warning in preview.warnings" :key="warning" class="warning-text">{{ warning }}</p>
        <details v-if="preview.examples?.length" class="naming-examples">
          <summary>查看目标路径{{ preview.examplesTruncated ? '（显示前 100 项）' : '' }}</summary>
          <div v-for="item in preview.examples" :key="item.assetId" class="naming-example">
            <span>{{ item.sourceRelativePath }}</span
            ><strong>→ {{ item.destinationRelativePath }}{{ item.skipped ? ' · 将跳过' : '' }}</strong>
          </div>
        </details>
      </div>
      <div class="notice">
        <Icon name="shield" />逐文件复制并校验 SHA-256，绝不覆盖已有文件。导出目录会附带 CSV 清单。
      </div></template
    ><template v-else
      ><div class="success-visual">
        <Icon :name="job?.status === 'completed' ? 'success' : 'export'" :size="42" />
        <h3>{{ job?.status === 'completed' ? '导出已完成' : '导出任务已提交' }}</h3>
        <p>在任务中心查看逐文件结果、校验信息和失败明细。</p>
        <ElProgress
          v-if="job?.progressTotal"
          :percentage="Math.round((job.progressDone / job.progressTotal) * 100)"
        /></div></template
    ><template #footer
      ><button class="btn" @click="app.exportOpen = false">{{ jobId ? '关闭' : '取消' }}</button
      ><RouterLink v-if="jobId" class="btn primary" to="/jobs" @click="app.exportOpen = false"
        >查看任务结果</RouterLink
      ><button
        v-else-if="!preview"
        class="btn primary"
        :disabled="!destination || busy || !selectedCount"
        @click="plan"
      >
        {{ busy ? '预演中…' : '预演导出' }}</button
      ><button v-else class="btn primary" :disabled="busy" @click="start">
        确认复制 {{ preview.fileCount }} 个媒体{{ preview.xmpCount ? ` + ${preview.xmpCount} 个 XMP` : '' }}
      </button></template
    ></ElDialog
  >
</template>
