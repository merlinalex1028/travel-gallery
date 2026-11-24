import { afterEach, expect, it, vi } from 'vitest';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { ElOption, ElSelect } from 'element-plus';
import CustomSelect from '../components/CustomSelect.vue';

let wrapper: VueWrapper;
afterEach(() => wrapper?.unmount());
function setup(props: Record<string, unknown> = {}) {
  wrapper = mount(CustomSelect, {
    attachTo: document.body,
    props: {
      options: [
        { label: 'JPG', value: 'jpg' },
        { label: '不可选', value: 'disabled', disabled: true },
        { label: 'PNG', value: 'png' },
      ],
      'aria-label': '文件格式',
      ...props,
    },
    global: { components: { ElSelect, ElOption } },
  });
  return wrapper.get('[role="combobox"]');
}
async function key(value: string, code = value) {
  await wrapper.get('[role="combobox"]').trigger('keydown', { key: value, code });
  await flushPromises();
}
function option(label: string) {
  return [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(
    (item) => item.textContent?.trim() === label,
  )!;
}
async function choose(label: string) {
  option(label).click();
  await flushPromises();
  const updates = wrapper.emitted('update:modelValue')!;
  await wrapper.setProps({ modelValue: updates.at(-1)![0] });
}

it('selects a single value by keyboard, skips disabled options and closes with Escape', async () => {
  const input = setup();
  expect(input.attributes('aria-label')).toBe('文件格式');
  await input.trigger('focus');
  await key('Enter');
  expect(input.attributes('aria-expanded')).toBe('true');
  await key('Home');
  await key('ArrowDown');
  await key('Enter');
  expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual(['png']);
  await wrapper.setProps({ modelValue: 'png' });
  expect(wrapper.text()).toContain('PNG');
  expect(input.attributes('aria-expanded')).toBe('false');
  await key('Enter');
  await key('Escape');
  expect(input.attributes('aria-expanded')).toBe('false');
  expect(document.activeElement).toBe(input.element);
});

it('toggles multiple values, reports the count and keeps the popup open', async () => {
  const input = setup({ modelValue: [], multiple: true });
  expect(wrapper.text()).toContain('请选择');
  await key(' ', 'Space');
  expect(input.attributes('aria-expanded')).toBe('true');
  expect(
    document.getElementById(input.attributes('aria-controls')!)?.getAttribute('aria-multiselectable'),
  ).toBe('true');
  await choose('JPG');
  expect(wrapper.get('[role="status"]').text()).toBe('已选 1 项');
  expect(option('JPG').getAttribute('aria-selected')).toBe('true');
  await choose('PNG');
  expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([['jpg', 'png']]);
  expect(wrapper.get('[role="status"]').text()).toBe('已选 2 项');
  expect(wrapper.get('[role="status"]').attributes('title')).toBe('JPG、PNG');
  expect(input.attributes('aria-expanded')).toBe('true');
  await key('Home');
  await key(' ', 'Space');
  expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([['png']]);
});

it('closes on an outside click without changing the selection', async () => {
  const input = setup({ modelValue: ['jpg'], multiple: true });
  await key('Enter');
  document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
  document.body.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
  await flushPromises();
  expect(input.attributes('aria-expanded')).toBe('false');
  expect(wrapper.emitted('update:modelValue')).toBeUndefined();
});

it('preserves free-form camera models including spaces', async () => {
  const input = setup({ modelValue: [], multiple: true, allowCreate: true, options: [] });
  await input.trigger('focus');
  await key('Enter');
  await input.setValue('Canon EOS R5');
  await vi.waitFor(() => expect(option('Canon EOS R5')).toBeDefined());
  const space = new KeyboardEvent('keydown', { key: ' ', code: 'Space', bubbles: true, cancelable: true });
  input.element.dispatchEvent(space);
  expect(space.defaultPrevented).toBe(false);
  await key('ArrowDown');
  await key('Enter');
  expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([['Canon EOS R5']]);
});

it('forwards and clears validation details on the focusable combobox', async () => {
  const input = setup({ ariaInvalid: true, ariaDescribedby: 'formats-error' });
  expect(input.attributes('aria-invalid')).toBe('true');
  expect(input.attributes('aria-describedby')).toBe('formats-error');
  await wrapper.setProps({ ariaInvalid: false, ariaDescribedby: undefined });
  expect(input.attributes('aria-invalid')).toBeUndefined();
  expect(input.attributes('aria-describedby')).toBeUndefined();
});

it('does not open or change a disabled select', async () => {
  const input = setup({ modelValue: [], multiple: true, disabled: true });
  expect(input.attributes('disabled')).toBeDefined();
  await key(' ', 'Space');
  await key('Enter');
  expect(input.attributes('aria-expanded')).toBe('false');
  expect(wrapper.emitted('update:modelValue')).toBeUndefined();
});
