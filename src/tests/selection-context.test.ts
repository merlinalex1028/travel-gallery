import { beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount } from '@vue/test-utils';
import SelectionBar from '../features/SelectionBar.vue';
import CustomSelect from '../components/CustomSelect.vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { date } from '../services/format';
import type { AssetSummary } from '../services/contracts';

vi.mock('../services/api', () => ({ api: vi.fn(), errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }));
beforeEach(() => setActivePinia(createPinia()));
const options = {
  global: {
    stubs: {
      RouterLink: true,
      Icon: true,
      ElDropdown: true,
      ElDropdownMenu: true,
      ElDropdownItem: true,
      ElDialog: true,
      ElSelect: true,
      ElOption: true,
    },
  },
};
it('shows the frozen range creation and expiry instants while the current query changes', () => {
  const lib = useLibrary();
  const createdAt = Date.UTC(2026, 9, 26, 8, 0);
  lib.snapshot = {
    id: 'original-jpegs',
    count: 2,
    createdAt,
    expiresAt: createdAt + 86400000,
    queryFingerprint: 'jpeg',
  };
  lib.fingerprint = 'heic';
  lib.total = 1;
  const target = { ...lib.target };
  const wrapper = mount(SelectionBar, options);
  expect(wrapper.get('time[aria-label="选区冻结时间"]').attributes('datetime')).toBe(
    new Date(createdAt).toISOString(),
  );
  expect(wrapper.get('time[aria-label="选区有效期"]').attributes('datetime')).toBe(
    new Date(createdAt + 86400000).toISOString(),
  );
  expect(wrapper.text()).toContain(date(createdAt, true));
  expect(wrapper.text()).toContain('2 张的冻结选区保持原范围');
  expect(lib.target).toEqual(target);
  wrapper.unmount();
});
it('keeps explicit selections free of snapshot timestamps', () => {
  const lib = useLibrary();
  lib.selected = new Set(['one']);
  const wrapper = mount(SelectionBar, options);
  expect(wrapper.find('time').exists()).toBe(false);
  expect(wrapper.text()).not.toContain('冻结于');
  wrapper.unmount();
});
it.each(['loading', 'error'] as const)(
  'disables new query selections while %s and keeps the frozen target visible',
  (state) => {
    const lib = useLibrary();
    lib.items = [{ id: 'old-result' } as AssetSummary];
    lib.total = 1;
    lib.snapshot = { id: 'retained', count: 2, createdAt: 1, expiresAt: 100, queryFingerprint: 'old' };
    if (state === 'loading') lib.loading = true;
    else lib.error = '范围起点不能大于终点';
    const target = { ...lib.target };
    const wrapper = mount(SelectionBar, options);
    const loaded = wrapper.findAll('button').find((button) => button.text().includes('选择当前已加载'))!;
    const all = wrapper.findAll('button').find((button) => button.text().includes('选择全部匹配'))!;
    expect(loaded.attributes('disabled')).toBeDefined();
    expect(all.attributes('disabled')).toBeDefined();
    expect(wrapper.text()).not.toContain('当前查询 1 张');
    expect(wrapper.text()).toContain(state === 'loading' ? '正在更新匹配结果' : '查询未完成');
    expect(wrapper.text()).toContain('已选 2 张');
    expect(lib.target).toEqual(target);
    wrapper.unmount();
  },
);

it.each(['add', 'remove'])('saves custom tag choices in %s mode for the current selection', async (mode) => {
  const lib = useLibrary();
  lib.selected = new Set(['one', 'two']);
  const apply = vi.spyOn(lib, 'applyTags').mockResolvedValue({
    changeBatchId: 'batch',
    affectedCount: 2,
    unchangedCount: 0,
    conflicts: [],
  });
  const app = useApp();
  app.tags = [{ id: 'sunset', name: '日落', color: null, count: 2 }];
  vi.spyOn(app, 'refresh').mockResolvedValue();
  const wrapper = mount(SelectionBar, {
    global: {
      stubs: {
        ...options.global.stubs,
        ElDialog: {
          props: ['modelValue', 'title'],
          template: '<section v-if="modelValue"><h2>{{ title }}</h2><slot/><slot name="footer"/></section>',
        },
      },
    },
  });
  try {
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '标签')!
      .trigger('click');
    const select = wrapper.getComponent(CustomSelect);
    expect(select.props()).toMatchObject({ options: [{ value: 'sunset', label: '日落' }] });
    select.vm.$emit('update:modelValue', ['sunset']);
    await wrapper.get('[aria-label="标签操作"]').setValue(mode);
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '保存标签')!
      .trigger('click');
    await flushPromises();
    expect(apply).toHaveBeenCalledWith(mode === 'add' ? ['sunset'] : [], mode === 'remove' ? ['sunset'] : []);
    expect(wrapper.text()).not.toContain('批量编辑标签');
  } finally {
    wrapper.unmount();
  }
});
