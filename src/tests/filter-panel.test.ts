import { selectControl, setSelect } from './helpers/select';
import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount } from '@vue/test-utils';
import FilterPanel from '../features/FilterPanel.vue';
import type { AssetFilter } from '../services/contracts';

vi.mock('element-plus', () => ({ ElMessage: { error: vi.fn() } }));

const options = {
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
};

beforeEach(() => setActivePinia(createPinia()));

it('shows and preserves existing conditions when a route opens the panel on mount', async () => {
  const filter: AssetFilter = {
    pickStates: ['keep'],
    mediaKinds: ['image'],
    ratingMin: 4,
    favorite: true,
    captureLocalFrom: '2024-01-01',
    captureLocalTo: '2024-12-31',
    tagRule: { mode: 'all', tagIds: ['travel', 'sunset'] },
  };
  const wrapper = mount(FilterPanel, { ...options, props: { filter, modelValue: true } });
  const checked = wrapper.findAll<HTMLInputElement>('input[type="checkbox"]:checked');
  expect(checked.map((input) => input.element.parentElement!.textContent!.trim())).toEqual(['保留', '照片']);
  expect(selectControl(wrapper, '最低星级').props()).toMatchObject({ modelValue: 4 });
  expect(wrapper.get<HTMLInputElement>('[aria-label="拍摄起始日期"]').element.value).toBe('2024-01-01');
  await wrapper
    .findAll('button')
    .find((button) => button.text() === '应用筛选')!
    .trigger('click');
  expect(wrapper.emitted('apply')![0]).toEqual([filter]);
  wrapper.unmount();
});

it('discards an unapplied draft and reads the new conditions on reopening', async () => {
  const original: AssetFilter = { ratingMin: 4, tagRule: { mode: 'any', tagIds: ['travel'] } };
  const wrapper = mount(FilterPanel, { ...options, props: { filter: original, modelValue: false } });
  await wrapper.setProps({ modelValue: true });
  await setSelect(wrapper, '最低星级', 2);
  expect(original.ratingMin).toBe(4);
  await wrapper.setProps({ modelValue: false });
  const replacement: AssetFilter = { ratingMax: 3, tagRule: { mode: 'exclude', tagIds: ['work'] } };
  await wrapper.setProps({ filter: replacement, modelValue: true });
  expect(selectControl(wrapper, '最高星级').props()).toMatchObject({ modelValue: 3 });
  await wrapper
    .findAll('button')
    .find((button) => button.text() === '应用筛选')!
    .trigger('click');
  expect(wrapper.emitted('apply')![0]).toEqual([replacement]);
  expect(original).toEqual({ ratingMin: 4, tagRule: { mode: 'any', tagIds: ['travel'] } });
  wrapper.unmount();
});

it('applies custom format, free-form camera and tag selections without mutating the active filter', async () => {
  const filter: AssetFilter = { keyword: '旅行' };
  const wrapper = mount(FilterPanel, { ...options, props: { filter, modelValue: true } });
  try {
    const formats = selectControl(wrapper, '文件格式（多选）'),
      cameras = selectControl(wrapper, '设备型号'),
      tags = selectControl(wrapper, '选择标签');
    expect(formats!.props()).toHaveProperty('options.length', 16);
    expect(cameras!.props()).toHaveProperty('allowCreate', true);
    formats!.vm.$emit('update:modelValue', ['jpg', 'heic']);
    cameras!.vm.$emit('update:modelValue', ['Canon EOS R5']);
    tags!.vm.$emit('update:modelValue', ['sunset', 'travel']);
    await wrapper.vm.$nextTick();
    expect(filter).toEqual({ keyword: '旅行' });
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '应用筛选')!
      .trigger('click');
    expect(wrapper.emitted('apply')![0]).toEqual([
      {
        keyword: '旅行',
        extensions: ['jpg', 'heic'],
        cameraModels: ['Canon EOS R5'],
        tagRule: { mode: 'any', tagIds: ['sunset', 'travel'] },
      },
    ]);
  } finally {
    wrapper.unmount();
  }
});
