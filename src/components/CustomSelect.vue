<script
  setup
  lang="ts"
  generic="T extends string | number | boolean | (string | number | boolean)[] | null | undefined"
>
import { computed, nextTick, useTemplateRef, watchPostEffect } from 'vue';
import Icon from './Icon.vue';

const model = defineModel<T>();
const props = withDefaults(
  defineProps<{
    options?: { value: string | number | boolean | null | undefined; label: string; disabled?: boolean }[];
    placeholder?: string;
    multiple?: boolean;
    disabled?: boolean;
    filterable?: boolean;
    allowCreate?: boolean;
    ariaLabel?: string;
    ariaInvalid?: boolean;
    ariaDescribedby?: string;
  }>(),
  { options: () => [], placeholder: '请选择' },
);
const emit = defineEmits<{ change: [value: T] }>();
const select = useTemplateRef<{ $el: HTMLElement }>('select');
// ElOption accepts objects but not null/undefined. Keep optional filters distinct from false, 0 and ''.
const unsetValue = { value: 'unset' },
  nullValue = { value: 'null' };
function encode(value: string | number | boolean | null | undefined) {
  return value === undefined ? unsetValue : value === null ? nullValue : value;
}
function decode(value: unknown) {
  return (value === unsetValue ? undefined : value === nullValue ? null : value) as T;
}
const internal = computed({
  get: () => {
    if (props.multiple) return Array.isArray(model.value) ? model.value : [];
    if (model.value == null && !props.options.some((option) => option.value === model.value))
      return undefined;
    return encode(model.value as string | number | boolean | null | undefined);
  },
  set: (value) => {
    model.value = decode(value);
  },
});
const selected = computed<(string | number | boolean)[]>(() =>
  Array.isArray(model.value) ? model.value : [],
);
const selectedLabels = computed(() =>
  selected.value
    .map((value) => props.options.find((option) => option.value === value)?.label ?? value)
    .join('、'),
);

// Element Plus forwards these attributes to its root; validation belongs on the combobox too.
watchPostEffect(() => {
  const input = select.value?.$el.querySelector('[role="combobox"]');
  for (const [name, value] of [
    ['aria-invalid', props.ariaInvalid ? 'true' : undefined],
    ['aria-describedby', props.ariaDescribedby],
  ]) {
    if (value) input?.setAttribute(name!, value);
    else input?.removeAttribute(name!);
  }
});

function handleSpace(event: KeyboardEvent) {
  if (event.key !== ' ' || props.filterable || props.allowCreate || props.disabled) return;
  event.preventDefault();
  event.stopPropagation();
  // Use the same supported navigation path as Enter; editable fields keep their spaces.
  (event.target as HTMLElement).dispatchEvent(
    new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }),
  );
}

async function describeListbox(open: boolean) {
  if (!open) return;
  await nextTick();
  const id = select.value?.$el.querySelector('[role="combobox"]')?.getAttribute('aria-controls');
  if (id && props.multiple) document.getElementById(id)?.setAttribute('aria-multiselectable', 'true');
}
</script>

<template>
  <ElSelect
    ref="select"
    v-model="internal"
    class="custom-select"
    :multiple="multiple"
    :disabled="disabled"
    :placeholder="placeholder"
    :filterable="filterable || allowCreate"
    :allow-create="allowCreate"
    :default-first-option="allowCreate"
    :reserve-keyword="false"
    :empty-values="[undefined]"
    :show-arrow="false"
    :aria-label="ariaLabel"
    :aria-invalid="ariaInvalid || undefined"
    :aria-describedby="ariaDescribedby"
    popper-class="custom-select-dropdown"
    no-data-text="暂无选项"
    no-match-text="没有匹配的选项"
    @keydown.capture="handleSpace"
    @visible-change="describeListbox"
    @change="emit('change', decode($event))"
  >
    <template #header>
      <div class="custom-select-heading" aria-hidden="true">
        <span>{{ ariaLabel || placeholder }}</span>
        <span class="custom-select-mode">{{ multiple ? '可多选' : '选择一项' }}</span>
      </div>
    </template>
    <template v-if="multiple" #tag>
      <span
        v-if="selected.length"
        class="custom-select-summary"
        :title="selectedLabels"
        role="status"
        aria-live="polite"
      >
        已选 {{ selected.length }} 项
      </span>
    </template>
    <ElOption
      v-for="option in options"
      :key="`${typeof option.value}:${String(option.value)}`"
      :value="encode(option.value)"
      :label="option.label"
      :disabled="option.disabled"
    >
      <span
        class="custom-select-marker"
        :class="{
          multiple,
          checked: multiple
            ? selected.includes(option.value as string | number | boolean)
            : model === option.value,
        }"
        aria-hidden="true"
        ><Icon name="check" :size="12"
      /></span>
      <span class="custom-select-option-label">{{ option.label }}</span>
    </ElOption>
  </ElSelect>
</template>

<style scoped>
.custom-select {
  width: var(--select-width, 100%);
  min-width: 0;
}
.custom-select :deep(.el-select__wrapper) {
  min-height: var(--select-height, 40px);
  padding: var(--select-padding, 10px 14px);
  border-radius: var(--r-sm);
  background: var(--surface);
  box-shadow:
    inset 0 0 0 var(--bw) var(--line),
    var(--shadow-btn);
  font: 700 var(--select-font-size, 13px) / 20px var(--font);
  transition:
    background-color 150ms,
    box-shadow 150ms,
    transform 150ms;
}
.custom-select :deep(.el-select__wrapper:hover:not(.is-disabled)) {
  background: var(--accent-soft);
  box-shadow:
    inset 0 0 0 var(--bw) var(--accent),
    var(--shadow);
  transform: translateY(-1px);
}
.custom-select :deep(.el-select__wrapper.is-focused) {
  background: var(--surface);
  box-shadow:
    inset 0 0 0 var(--bw) var(--accent),
    0 0 0 4px var(--focus-ring);
}
.custom-select[aria-invalid='true'] :deep(.el-select__wrapper) {
  box-shadow:
    inset 0 0 0 var(--bw) var(--danger),
    0 0 0 3px var(--danger-bg);
}
.custom-select :deep(.el-select__wrapper.is-disabled) {
  background: var(--sidebar);
  opacity: 0.55;
  cursor: not-allowed;
}
.custom-select :deep(.el-select__placeholder),
.custom-select :deep(.el-select__input),
.custom-select-summary {
  color: var(--text);
  font: inherit;
}
.custom-select :deep(.el-select__placeholder.is-transparent) {
  color: var(--muted);
}
.custom-select :deep(.el-select__input) {
  width: 100%;
  min-height: 0;
  padding: 0;
  border: 0;
  border-radius: 0;
  outline: none;
  background: transparent;
  box-shadow: none;
  letter-spacing: normal;
}
.custom-select :deep(.el-select__caret) {
  color: var(--accent-ink);
}
.custom-select-summary {
  max-width: 100%;
  padding: 0 8px;
  border-radius: var(--r-xs);
  background: var(--accent-soft);
  color: var(--accent-ink);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
@media (prefers-reduced-motion: reduce) {
  .custom-select :deep(.el-select__wrapper) {
    transition: none;
    transform: none !important;
  }
}
</style>
