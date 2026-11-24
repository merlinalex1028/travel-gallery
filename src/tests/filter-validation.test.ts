import { setSelect } from './helpers/select';
import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import FilterPanel from '../features/FilterPanel.vue';
import type { AssetFilter } from '../services/contracts';

vi.mock('element-plus', () => ({ ElMessage: { error: vi.fn() } }));
beforeEach(() => setActivePinia(createPinia()));
function panel(filter: AssetFilter = { keyword: 'HEIC' }) {
  return mount(FilterPanel, {
    props: { filter, modelValue: true },
    global: {
      stubs: {
        ElDrawer: {
          props: ['modelValue'],
          template: '<section v-if="modelValue"><slot/><slot name="footer"/></section>',
        },
        ElSelect: { template: '<div><slot/></div>' },
        ElOption: true,
      },
    },
  });
}
function applyButton(wrapper: ReturnType<typeof panel>) {
  return wrapper.findAll('button').find((button) => button.text() === '应用筛选')!;
}
function describedError(wrapper: ReturnType<typeof panel>, label: string) {
  const input = wrapper.get(`[aria-label="${label}"]`);
  expect(input.attributes('aria-invalid')).toBe('true');
  const id = input.attributes('aria-describedby');
  expect(id).toBeTruthy();
  return wrapper.get(`[id="${id}"]`).text();
}

it.each([
  ['拍摄起始日期', '拍摄结束日期'],
  ['导入起始日期', '导入结束日期'],
] as const)(
  'keeps a reversed %s draft out of the active query and clears its errors after correction',
  async (from, to) => {
    const filter = { keyword: 'HEIC' };
    const wrapper = panel(filter);
    await wrapper.get(`[aria-label="${from}"]`).setValue('2024-03-24');
    await wrapper.get(`[aria-label="${to}"]`).setValue('2024-03-23');
    expect(describedError(wrapper, from)).toContain('起始日期不能晚于结束日期');
    expect(describedError(wrapper, to)).toContain('起始日期不能晚于结束日期');
    expect(applyButton(wrapper).attributes('disabled')).toBeDefined();
    await applyButton(wrapper).trigger('click');
    expect(wrapper.emitted('apply')).toBeUndefined();
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
    expect(filter).toEqual({ keyword: 'HEIC' });
    await wrapper.get(`[aria-label="${to}"]`).setValue('2024-03-24');
    expect(wrapper.findAll('[aria-invalid="true"]')).toHaveLength(0);
    expect(applyButton(wrapper).attributes('disabled')).toBeUndefined();
    await applyButton(wrapper).trigger('click');
    const result = wrapper.emitted('apply')![0][0] as AssetFilter;
    expect(result.keyword).toBe('HEIC');
    if (from === '拍摄起始日期') {
      expect(result.captureLocalFrom).toBe('2024-03-24');
      expect(result.captureLocalTo).toBe('2024-03-24');
    } else {
      expect(result.indexedFromMs).toBe(new Date('2024-03-24T00:00:00').getTime());
      expect(result.indexedToMs).toBe(new Date('2024-03-24T23:59:59.999').getTime());
    }
    wrapper.unmount();
  },
);

it.each([
  ['最小文件大小', '最大文件大小', 'minBytes', 'maxBytes'],
  ['最短时长', '最长时长', 'durationMinMs', 'durationMaxMs'],
] as const)('rejects reversed %s ranges but accepts equal zero endpoints', async (from, to, min, max) => {
  const wrapper = panel();
  await wrapper.get(`[aria-label="${from}"]`).setValue('101');
  await wrapper.get(`[aria-label="${to}"]`).setValue('100');
  expect(describedError(wrapper, from)).toContain('最小值不能大于最大值');
  expect(describedError(wrapper, to)).toContain('最小值不能大于最大值');
  expect(applyButton(wrapper).attributes('disabled')).toBeDefined();
  await wrapper.get(`[aria-label="${from}"]`).setValue('0');
  await wrapper.get(`[aria-label="${to}"]`).setValue('0');
  expect(wrapper.findAll('[aria-invalid="true"]')).toHaveLength(0);
  await applyButton(wrapper).trigger('click');
  expect(wrapper.emitted('apply')![0][0]).toMatchObject({ [min]: 0, [max]: 0, keyword: 'HEIC' });
  wrapper.unmount();
});

it.each([
  ['最小文件大小', 'minBytes', '-1'],
  ['最短时长', 'durationMinMs', '0.5'],
  ['最小宽度', 'minWidth', '9007199254740992'],
] as const)('identifies an invalid %s without changing the active filter', async (label, field, invalid) => {
  const filter = { keyword: 'HEIC' };
  const wrapper = panel(filter);
  await wrapper.get(`[aria-label="${label}"]`).setValue(invalid);
  expect(describedError(wrapper, label)).toContain('请输入0或以上的有效整数');
  expect(applyButton(wrapper).attributes('disabled')).toBeDefined();
  expect(filter).toEqual({ keyword: 'HEIC' });
  await wrapper.get(`[aria-label="${label}"]`).setValue('0');
  expect(wrapper.findAll('[aria-invalid="true"]')).toHaveLength(0);
  await applyButton(wrapper).trigger('click');
  expect(wrapper.emitted('apply')![0][0]).toMatchObject({ [field]: 0 });
  wrapper.unmount();
});

it('associates a reversed star range with both star controls', async () => {
  const wrapper = panel({ ratingMin: 4, ratingMax: 2, favorite: true });
  expect(describedError(wrapper, '最低星级')).toContain('最低星级不能高于最高星级');
  expect(describedError(wrapper, '最高星级')).toContain('最低星级不能高于最高星级');
  await setSelect(wrapper, '最高星级', 4);
  await applyButton(wrapper).trigger('click');
  expect(wrapper.emitted('apply')![0][0]).toEqual({ ratingMin: 4, ratingMax: 4, favorite: true });
  wrapper.unmount();
});

it('clears invalid drafts and their descriptions when conditions are reset', async () => {
  const wrapper = panel({ minBytes: 10, maxBytes: 1 });
  expect(wrapper.findAll('[aria-invalid="true"]').length).toBeGreaterThan(0);
  await wrapper
    .findAll('button')
    .find((button) => button.text() === '清空条件')!
    .trigger('click');
  expect(wrapper.emitted('apply')![0]).toEqual([{}]);
  expect(wrapper.findAll('[aria-invalid="true"]')).toHaveLength(0);
  wrapper.unmount();
});

it('does not treat an incomplete browser number as an intentionally empty optional field', async () => {
  const wrapper = panel();
  const input = wrapper.get<HTMLInputElement>('[aria-label="最小文件大小"]');
  // Model the browser's badInput signal; native input handling is verified separately.
  let incomplete = true;
  Object.defineProperty(input.element, 'validity', { get: () => ({ badInput: incomplete }) });
  await input.trigger('input');
  expect(describedError(wrapper, '最小文件大小')).toContain('请输入0或以上的有效整数');
  expect(applyButton(wrapper).attributes('disabled')).toBeDefined();
  incomplete = false;
  await input.setValue('0');
  expect(wrapper.findAll('[aria-invalid="true"]')).toHaveLength(0);
  wrapper.unmount();
});

it('flags an invalid calendar date loaded from a saved rule', () => {
  const wrapper = panel({ captureLocalFrom: '2024-02-30' });
  expect(describedError(wrapper, '拍摄起始日期')).toContain('请输入有效日期');
  expect(applyButton(wrapper).attributes('disabled')).toBeDefined();
  wrapper.unmount();
});

it.each([
  ['sourceIds', '来源目录'],
  ['extensions', '文件格式（多选）'],
  ['cameraModels', '设备型号'],
] as const)('identifies too many %s choices in an existing rule', (field, label) => {
  const wrapper = panel({ [field]: Array.from({ length: 201 }, (_, n) => `choice-${n}`) });
  expect(describedError(wrapper, label)).toContain('最多选择200项');
  expect(applyButton(wrapper).attributes('disabled')).toBeDefined();
  wrapper.unmount();
});

it('identifies an oversized tag rule and permits clearing it', async () => {
  const wrapper = panel({
    tagRule: { mode: 'exclude', tagIds: Array.from({ length: 201 }, (_, n) => `tag-${n}`) },
  });
  expect(describedError(wrapper, '选择标签')).toContain('最多选择200项');
  await wrapper
    .findAll('button')
    .find((button) => button.text() === '清空条件')!
    .trigger('click');
  expect(wrapper.emitted('apply')![0]).toEqual([{}]);
  expect(wrapper.findAll('[aria-invalid="true"]')).toHaveLength(0);
  wrapper.unmount();
});
