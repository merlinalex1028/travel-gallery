<script setup lang="ts" generic="T extends string | number | boolean | (string | number | boolean)[] | null">
import { computed, useTemplateRef, watchPostEffect } from 'vue';

const model = defineModel<T>();
const props = withDefaults(
  defineProps<{
    options?: { value: string | number | boolean; label: string; disabled?: boolean }[];
    placeholder?: string;
    multiple?: boolean;
    disabled?: boolean;
    filterable?: boolean;
    allowCreate?: boolean;
    ariaInvalid?: boolean;
    ariaDescribedby?: string;
  }>(),
  { options: () => [], placeholder: '请选择' },
);
const select = useTemplateRef<{ $el: HTMLElement }>('select');
const selected = computed(() => (Array.isArray(model.value) ? model.value : []));
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
</script>

<template>
  <ElSelect
    ref="select"
    v-model="model"
    class="custom-select"
    :multiple="multiple"
    :disabled="disabled"
    :placeholder="placeholder"
    :filterable="filterable || allowCreate"
    :allow-create="allowCreate"
    :default-first-option="allowCreate"
    :reserve-keyword="false"
    :aria-invalid="ariaInvalid || undefined"
    :aria-describedby="ariaDescribedby"
    popper-class="custom-select-dropdown"
    no-data-text="暂无选项"
    no-match-text="没有匹配的选项"
    @keydown.capture="handleSpace"
  >
    <template v-if="multiple" #tag>
      <span class="custom-select-summary" :title="selectedLabels" role="status" aria-live="polite">
        已选 {{ selected.length }} 项
      </span>
    </template>
    <ElOption
      v-for="option in options"
      :key="String(option.value)"
      :value="option.value"
      :label="option.label"
      :disabled="option.disabled"
    />
  </ElSelect>
</template>

<style scoped>
.custom-select {
  width: 100%;
  min-width: 0;
}
.custom-select :deep(.el-select__wrapper) {
  min-height: 40px;
  padding: 10px 14px;
  border-radius: var(--r-sm);
  background: var(--surface);
  box-shadow:
    inset 0 0 0 var(--bw) var(--line),
    var(--shadow-btn);
  font: 700 13px / 20px var(--font);
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
