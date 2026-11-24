import { nextTick } from 'vue';
import type { VueWrapper } from '@vue/test-utils';

export function selectControl(wrapper: VueWrapper, label: string): VueWrapper {
  const control = wrapper
    .findAllComponents({ name: 'CustomSelect' })
    .find((item) => (item.props() as { ariaLabel?: string }).ariaLabel === label);
  if (!control) throw new Error(`Missing select: ${label}`);
  return control;
}

export async function setSelect(wrapper: VueWrapper, label: string, value: unknown) {
  const control = selectControl(wrapper, label);
  control.vm.$emit('update:modelValue', value);
  await nextTick();
  control.vm.$emit('change', value);
  await nextTick();
}
