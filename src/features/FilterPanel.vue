<script setup lang="ts">
import { computed, reactive, shallowRef, useId, watch } from 'vue';
import { useApp } from '../stores/app';
import type { AssetFilter, PickState, MediaKind, FileState } from '../services/contracts';
import { picks, files } from '../services/format';
import Icon from '../components/Icon.vue';
import CustomSelect from '../components/CustomSelect.vue';
const props = defineProps<{ filter: AssetFilter }>(),
  open = defineModel<boolean>({ default: false });
const emit = defineEmits<{ apply: [filter: AssetFilter] }>(),
  app = useApp();
const form = reactive<AssetFilter>({}),
  captureFrom = shallowRef(''),
  captureTo = shallowRef(''),
  indexedFrom = shallowRef(''),
  indexedTo = shallowRef(''),
  tagMode = shallowRef<'any' | 'all' | 'exclude'>('any'),
  tagIds = shallowRef<string[]>([]);
const pickOptions: PickState[] = ['unreviewed', 'keep', 'maybe', 'reject'],
  kindOptions: MediaKind[] = ['image', 'video'],
  fileOptions: FileState[] = ['available', 'offline', 'missing', 'unsupported', 'corrupt'];
const extensionOptions = [
  'jpg',
  'jpeg',
  'png',
  'webp',
  'heic',
  'heif',
  'tif',
  'tiff',
  'arw',
  'cr2',
  'cr3',
  'nef',
  'dng',
  'mov',
  'mp4',
  'webm',
].map((value) => ({ value, label: value.toUpperCase() }));
const tagOptions = computed(() => app.tags.map((tag) => ({ value: tag.id, label: tag.name })));
const cameraModelOptions = computed(() =>
  (form.cameraModels ?? []).map((value) => ({ value, label: value })),
);
const numericFields = [
  'ratingMin',
  'ratingMax',
  'minBytes',
  'maxBytes',
  'minWidth',
  'minHeight',
  'durationMinMs',
  'durationMaxMs',
] as const;
type NumericField = (typeof numericFields)[number];
type DateField = 'captureFrom' | 'captureTo' | 'indexedFrom' | 'indexedTo';
type DraftField = NumericField | DateField | 'sourceIds' | 'extensions' | 'cameraModels' | 'tagIds';
const badNumber = reactive<Partial<Record<NumericField, boolean>>>({});
const validationId = useId();
const errors = computed(() => {
  const result: Partial<Record<DraftField, string>> = {};
  for (const field of ['sourceIds', 'extensions', 'cameraModels'] as const) {
    if ((form[field]?.length ?? 0) > 200) result[field] = '最多选择200项，请移除多余选项';
  }
  if (tagIds.value.length > 200) result.tagIds = '最多选择200项，请移除多余选项';
  for (const field of numericFields) {
    const value: unknown = form[field];
    if (
      badNumber[field] ||
      (value !== undefined &&
        value !== null &&
        value !== '' &&
        (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0))
    )
      result[field] = '请输入0或以上的有效整数';
  }
  for (const field of ['ratingMin', 'ratingMax'] as const) {
    const value = form[field];
    if (typeof value === 'number' && (value < 0 || value > 5)) result[field] = '星级必须为0～5';
  }
  for (const [from, to, message] of [
    ['ratingMin', 'ratingMax', '最低星级不能高于最高星级'],
    ['minBytes', 'maxBytes', '最小值不能大于最大值'],
    ['durationMinMs', 'durationMaxMs', '最小值不能大于最大值'],
  ] as const) {
    const first = form[from],
      last = form[to];
    if (!result[from] && !result[to] && typeof first === 'number' && typeof last === 'number' && first > last)
      result[from] = result[to] = message;
  }
  const dates = {
    captureFrom: captureFrom.value,
    captureTo: captureTo.value,
    indexedFrom: indexedFrom.value,
    indexedTo: indexedTo.value,
  };
  for (const field of Object.keys(dates) as DateField[]) {
    const value = dates[field];
    if (value) {
      const date = new Date(`${value}T00:00:00Z`);
      if (
        !/^\d{4}-\d{2}-\d{2}$/.test(value) ||
        !Number.isFinite(date.getTime()) ||
        date.toISOString().slice(0, 10) !== value
      )
        result[field] = '请输入有效日期';
    }
  }
  for (const [from, to] of [
    ['captureFrom', 'captureTo'],
    ['indexedFrom', 'indexedTo'],
  ] as const) {
    if (!result[from] && !result[to] && dates[from] && dates[to] && dates[from] > dates[to])
      result[from] = result[to] = '起始日期不能晚于结束日期';
  }
  return result;
});
const hasErrors = computed(() => Object.keys(errors.value).length > 0);
function errorId(group: string) {
  return `${validationId}-${group}-error`;
}
function validationAttrs(field: DraftField, group: string = field) {
  return {
    'aria-invalid': errors.value[field] ? true : undefined,
    'aria-describedby': errors.value[field] ? errorId(group) : undefined,
  };
}
function issue(...fields: DraftField[]) {
  return [...new Set(fields.map((field) => errors.value[field]).filter(Boolean))].join('；');
}
function trackNumber(field: NumericField, event: Event) {
  badNumber[field] = (event.target as HTMLInputElement).validity.badInput;
}
watch(
  open,
  (value) => {
    if (value) {
      for (const field of numericFields) delete badNumber[field];
      for (const k of Object.keys(form)) delete form[k as keyof AssetFilter];
      Object.assign(form, JSON.parse(JSON.stringify(props.filter)));
      captureFrom.value =
        form.captureLocalFrom ?? (form.captureFromMs !== undefined ? localDate(form.captureFromMs) : '');
      captureTo.value =
        form.captureLocalTo ?? (form.captureToMs !== undefined ? localDate(form.captureToMs) : '');
      indexedFrom.value = form.indexedFromMs !== undefined ? localDate(form.indexedFromMs) : '';
      indexedTo.value = form.indexedToMs !== undefined ? localDate(form.indexedToMs) : '';
      tagIds.value = form.tagRule?.tagIds ?? [];
      tagMode.value = form.tagRule?.mode ?? 'any';
    }
  },
  { immediate: true },
);
function localDate(ms: number) {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}
function list(
  field: 'sourceIds' | 'pickStates' | 'mediaKinds' | 'extensions' | 'fileStates',
  value: string,
  checked: boolean,
) {
  const old = (form[field] ?? []) as string[];
  Object.assign(form, { [field]: checked ? [...old, value] : old.filter((v) => v !== value) });
}
function apply() {
  if (hasErrors.value) return;
  const value = JSON.parse(JSON.stringify(form)) as AssetFilter;
  delete value.captureFromMs;
  delete value.captureToMs;
  value.captureLocalFrom = captureFrom.value || undefined;
  value.captureLocalTo = captureTo.value || undefined;
  value.indexedFromMs = indexedFrom.value ? new Date(`${indexedFrom.value}T00:00:00`).getTime() : undefined;
  value.indexedToMs = indexedTo.value ? new Date(`${indexedTo.value}T23:59:59.999`).getTime() : undefined;
  value.tagRule = tagIds.value.length ? { mode: tagMode.value, tagIds: tagIds.value } : undefined;
  for (const [key, v] of Object.entries(value))
    if (v === '' || v === null || (Array.isArray(v) && !v.length)) delete value[key as keyof AssetFilter];
  emit('apply', value);
  open.value = false;
}
function reset() {
  for (const field of numericFields) delete badNumber[field];
  for (const k of Object.keys(form)) delete form[k as keyof AssetFilter];
  captureFrom.value = '';
  captureTo.value = '';
  indexedFrom.value = '';
  indexedTo.value = '';
  tagIds.value = [];
  emit('apply', {});
  open.value = false;
}
</script>
<template>
  <ElDrawer v-model="open" title="组合筛选" size="390px"
    ><div class="filter-explainer"><Icon name="filter" :size="16" />不同条件同时满足，同一条件内任一匹配</div>
    <div v-if="hasErrors" class="notice warning" role="alert">请先修正标红的筛选条件。</div>
    <div class="filter-section">
      <h4>挑片状态</h4>
      <div class="filter-options">
        <label v-for="s in pickOptions" :key="s" class="check-label"
          ><input
            type="checkbox"
            :checked="form.pickStates?.includes(s)"
            @change="list('pickStates', s, ($event.target as HTMLInputElement).checked)"
          />{{ picks[s] }}</label
        >
      </div>
    </div>
    <div class="filter-section">
      <h4>媒体类型</h4>
      <div class="filter-options">
        <label v-for="s in kindOptions" :key="s" class="check-label"
          ><input
            type="checkbox"
            :checked="form.mediaKinds?.includes(s)"
            @change="list('mediaKinds', s, ($event.target as HTMLInputElement).checked)"
          />{{ s === 'image' ? '照片' : '视频' }}</label
        >
      </div>
    </div>
    <div class="filter-section form-stack">
      <label
        >最低星级<CustomSelect
          v-model="form.ratingMin"
          aria-label="最低星级"
          v-bind="validationAttrs('ratingMin')"
          :options="[
            { value: undefined, label: '不限' },
            ...Array.from({ length: 6 }, (_, index) => {
              const n = index + 1;
              return { value: n - 1, label: String(n - 1) + ' 星及以上' };
            }),
          ]"
        /><small v-if="errors.ratingMin" :id="errorId('ratingMin')" class="filter-field-error" role="alert">{{
          errors.ratingMin
        }}</small></label
      ><label
        >最高星级<CustomSelect
          v-model="form.ratingMax"
          aria-label="最高星级"
          v-bind="validationAttrs('ratingMax')"
          :options="[
            { value: undefined, label: '不限' },
            ...Array.from({ length: 6 }, (_, index) => {
              const n = index + 1;
              return { value: n - 1, label: String(n - 1) + ' 星及以下' };
            }),
          ]"
        /><small v-if="errors.ratingMax" :id="errorId('ratingMax')" class="filter-field-error" role="alert">{{
          errors.ratingMax
        }}</small></label
      ><label
        >收藏<CustomSelect
          v-model="form.favorite"
          aria-label="收藏"
          :options="[
            { value: undefined, label: '全部' },
            { value: true, label: '仅收藏' },
            { value: false, label: '排除收藏' },
          ]"
      /></label>
    </div>
    <div class="filter-section">
      <h4>来源目录</h4>
      <div role="group" aria-label="来源目录" v-bind="validationAttrs('sourceIds')">
        <label v-for="s in app.sources" :key="s.id" class="check-label"
          ><input
            type="checkbox"
            :checked="form.sourceIds?.includes(s.id)"
            @change="list('sourceIds', s.id, ($event.target as HTMLInputElement).checked)"
          />{{ s.displayName }}</label
        >
      </div>
      <small v-if="errors.sourceIds" :id="errorId('sourceIds')" class="filter-field-error" role="alert">{{
        errors.sourceIds
      }}</small>
    </div>
    <div class="filter-section form-stack">
      <label>目录片段<input v-model="form.folder" placeholder="相对来源的目录前缀" /></label
      ><label
        >拍摄日期可信度<CustomSelect
          v-model="form.captureDateKnown"
          aria-label="拍摄日期可信度"
          :options="[
            { value: undefined, label: '全部' },
            { value: true, label: '有拍摄日期' },
            { value: false, label: '未知日期 / 仅文件修改时间' },
          ]" /></label
      ><label
        >拍摄日期
        <div class="input-pair">
          <input
            v-model="captureFrom"
            type="date"
            aria-label="拍摄起始日期"
            v-bind="validationAttrs('captureFrom', 'capture')"
          /><input
            v-model="captureTo"
            type="date"
            aria-label="拍摄结束日期"
            v-bind="validationAttrs('captureTo', 'capture')"
          />
        </div>
        <small
          v-if="issue('captureFrom', 'captureTo')"
          :id="errorId('capture')"
          class="filter-field-error"
          role="alert"
          >{{ issue('captureFrom', 'captureTo') }}</small
        ></label
      ><label
        >导入日期
        <div class="input-pair">
          <input
            v-model="indexedFrom"
            type="date"
            aria-label="导入起始日期"
            v-bind="validationAttrs('indexedFrom', 'indexed')"
          /><input
            v-model="indexedTo"
            type="date"
            aria-label="导入结束日期"
            v-bind="validationAttrs('indexedTo', 'indexed')"
          />
        </div>
        <small
          v-if="issue('indexedFrom', 'indexedTo')"
          :id="errorId('indexed')"
          class="filter-field-error"
          role="alert"
          >{{ issue('indexedFrom', 'indexedTo') }}</small
        ></label
      >
    </div>
    <div class="filter-section form-stack">
      <label
        >文件格式（多选）
        <div role="group" aria-label="文件格式（多选）" v-bind="validationAttrs('extensions')">
          <CustomSelect
            v-model="form.extensions"
            multiple
            placeholder="全部格式"
            aria-label="文件格式（多选）"
            :options="extensionOptions"
            v-bind="validationAttrs('extensions')"
          />
        </div>
        <small v-if="errors.extensions" :id="errorId('extensions')" class="filter-field-error" role="alert">{{
          errors.extensions
        }}</small></label
      ><label
        >设备型号
        <div role="group" aria-label="设备型号" v-bind="validationAttrs('cameraModels')">
          <CustomSelect
            v-model="form.cameraModels"
            multiple
            filterable
            allow-create
            aria-label="设备型号"
            placeholder="输入 EXIF 中的完整型号"
            :options="cameraModelOptions"
            v-bind="validationAttrs('cameraModels')"
          />
        </div>
        <small
          v-if="errors.cameraModels"
          :id="errorId('cameraModels')"
          class="filter-field-error"
          role="alert"
          >{{ errors.cameraModels }}</small
        ></label
      ><label
        >文件大小范围（字节）
        <div class="input-pair">
          <input
            v-model.number="form.minBytes"
            type="number"
            min="0"
            placeholder="最小"
            aria-label="最小文件大小"
            v-bind="validationAttrs('minBytes', 'bytes')"
            @input="trackNumber('minBytes', $event)"
          /><input
            v-model.number="form.maxBytes"
            type="number"
            min="0"
            placeholder="最大"
            aria-label="最大文件大小"
            v-bind="validationAttrs('maxBytes', 'bytes')"
            @input="trackNumber('maxBytes', $event)"
          />
        </div>
        <small
          v-if="issue('minBytes', 'maxBytes')"
          :id="errorId('bytes')"
          class="filter-field-error"
          role="alert"
          >{{ issue('minBytes', 'maxBytes') }}</small
        ></label
      ><label
        >最低分辨率
        <div class="input-pair">
          <input
            v-model.number="form.minWidth"
            type="number"
            min="0"
            placeholder="宽度"
            aria-label="最小宽度"
            v-bind="validationAttrs('minWidth', 'resolution')"
            @input="trackNumber('minWidth', $event)"
          /><input
            v-model.number="form.minHeight"
            type="number"
            min="0"
            placeholder="高度"
            aria-label="最小高度"
            v-bind="validationAttrs('minHeight', 'resolution')"
            @input="trackNumber('minHeight', $event)"
          />
        </div>
        <small
          v-if="issue('minWidth', 'minHeight')"
          :id="errorId('resolution')"
          class="filter-field-error"
          role="alert"
          >{{ issue('minWidth', 'minHeight') }}</small
        ></label
      ><label
        >视频时长范围（毫秒）
        <div class="input-pair">
          <input
            v-model.number="form.durationMinMs"
            type="number"
            min="0"
            placeholder="最短"
            aria-label="最短时长"
            v-bind="validationAttrs('durationMinMs', 'duration')"
            @input="trackNumber('durationMinMs', $event)"
          /><input
            v-model.number="form.durationMaxMs"
            type="number"
            min="0"
            placeholder="最长"
            aria-label="最长时长"
            v-bind="validationAttrs('durationMaxMs', 'duration')"
            @input="trackNumber('durationMaxMs', $event)"
          />
        </div>
        <small
          v-if="issue('durationMinMs', 'durationMaxMs')"
          :id="errorId('duration')"
          class="filter-field-error"
          role="alert"
          >{{ issue('durationMinMs', 'durationMaxMs') }}</small
        ></label
      >
    </div>
    <div class="filter-section form-stack">
      <label
        >标签规则<CustomSelect
          v-model="tagMode"
          aria-label="标签规则"
          :options="[
            { value: 'any', label: '包含任一标签' },
            { value: 'all', label: '包含全部标签' },
            { value: 'exclude', label: '排除这些标签' },
          ]"
      /></label>
      <div role="group" aria-label="选择标签" v-bind="validationAttrs('tagIds')">
        <CustomSelect
          v-model="tagIds"
          multiple
          placeholder="选择标签"
          aria-label="选择标签"
          :options="tagOptions"
          v-bind="validationAttrs('tagIds')"
        />
      </div>
      <small v-if="errors.tagIds" :id="errorId('tagIds')" class="filter-field-error" role="alert">{{
        errors.tagIds
      }}</small>
    </div>
    <div class="filter-section">
      <h4>文件状态</h4>
      <label v-for="s in fileOptions" :key="s" class="check-label"
        ><input
          type="checkbox"
          :checked="form.fileStates?.includes(s)"
          @change="list('fileStates', s, ($event.target as HTMLInputElement).checked)"
        />{{ files[s] }}</label
      >
    </div>
    <div class="filter-section form-stack">
      <label
        >重复分组<CustomSelect
          v-model="form.duplicateKind"
          aria-label="重复分组"
          :options="[
            { value: undefined, label: '不限' },
            { value: 'exact', label: '精确重复组' },
            { value: 'similar', label: '视觉相似组' },
            { value: 'any', label: '任一分组' },
            { value: 'none', label: '未加入重复组' },
          ]"
      /></label>
    </div>
    <template #footer
      ><button class="btn" @click="reset">清空条件</button
      ><button class="btn primary" :disabled="hasErrors" @click="apply">应用筛选</button></template
    ></ElDrawer
  >
</template>
<style scoped>
.filter-section [aria-invalid='true'] {
  border-color: var(--danger);
  box-shadow: 0 0 0 1px var(--danger);
}
.filter-field-error {
  color: var(--danger);
  font-size: 12px;
  line-height: 1.5;
}
</style>
