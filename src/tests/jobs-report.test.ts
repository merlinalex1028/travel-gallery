import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import JobsPage from '../pages/JobsPage.vue';
import { useApp } from '../stores/app';
import type { Commands, JobSummary } from '../services/contracts';
import golden from '../../tests/fixtures/contracts.json';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../services/api', () => ({ api: invoke, errorMessage: () => 'failed' }));
vi.mock('element-plus', () => ({ ElMessage: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }));
let wrapper: VueWrapper | undefined;
beforeEach(() => {
  setActivePinia(createPinia());
  invoke.mockReset();
});
afterEach(() => wrapper?.unmount());
async function showReport(status: 'cancelled' | 'running') {
  const report: Commands['get_export_report']['result'] = {
    ...golden.exportReport,
    jobId: 'cancelled-export',
    status,
    copiedCount: 424,
    verifiedCount: 424,
    failedCount: 1,
    skippedCount: 0,
    manifestPath: null,
    sidecars: [],
    xmpFailedCount: 0,
    items: Array.from({ length: 500 }, (_, i) => ({
      assetId: `asset-${i}`,
      destination: `photo-${i}.jpg`,
      state: i < 424 ? 'copied' : i === 424 ? 'failed' : 'queued',
      errorCode: i === 424 ? 'CANCELLED' : null,
      sha256: i < 424 ? 'verified-source-hash' : null,
    })),
  };
  invoke.mockImplementation(async (name: string) => {
    if (name === 'get_export_report') return report;
    throw new Error(name);
  });
  useApp().jobs = [
    {
      id: report.jobId,
      type: 'EXPORT',
      status,
      progressDone: 424,
      progressTotal: 500,
      failureCount: 0,
      errorSummary: null,
      createdAt: 1,
      updatedAt: 2,
    } as JobSummary,
  ];
  wrapper = mount(JobsPage, {
    global: {
      stubs: {
        Icon: true,
        EmptyState: true,
        ElProgress: true,
        RouterLink: { template: '<span><slot /></span>' },
        ElDialog: {
          props: ['modelValue', 'title'],
          template: '<section v-if="modelValue" role="dialog" :aria-label="title"><slot /></section>',
        },
      },
    },
  });
  await wrapper
    .findAll('button')
    .find((button) => button.text() === '导出报告')!
    .trigger('click');
  await flushPromises();
  return wrapper;
}
it('accounts for unstarted media after cancellation and bounds the report DOM while retaining every row', async () => {
  const wrapper = await showReport('cancelled');
  expect(wrapper.get('[aria-label="未完成媒体"]').text()).toContain('75');
  expect(wrapper.text()).toContain('不会自动继续');
  expect(wrapper.text()).toContain('重新预演');
  expect(wrapper.findAll('.report-table > div')).toHaveLength(100);
  for (let page = 0; page < 4; page++)
    await wrapper
      .findAll('button')
      .find((button) => button.text() === '下一页')!
      .trigger('click');
  expect(wrapper.findAll('.report-table > div')).toHaveLength(100);
  expect(wrapper.text()).toContain('photo-499.jpg');
  expect(wrapper.text()).toContain('未开始（任务已取消）');
  expect(wrapper.text()).not.toContain('等待处理');
  expect(
    wrapper
      .findAll('button')
      .find((button) => button.text() === '下一页')!
      .attributes('disabled'),
  ).toBeDefined();
});
it('keeps a running report distinct from stopped work', async () => {
  const wrapper = await showReport('running');
  expect(wrapper.get('[aria-label="未完成媒体"]').text()).toContain('75');
  expect(wrapper.text()).toContain('任务仍在进行');
  expect(wrapper.text()).not.toContain('不会自动继续');
});
