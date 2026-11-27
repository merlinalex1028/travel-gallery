import { afterEach, describe, expect, it, vi } from 'vitest';
import { computed } from 'vue';
import { mount } from '@vue/test-utils';
import { errorText, locale, setLocale, t, tr, translatedRecord } from '../i18n';
import LanguageSwitch from '../components/LanguageSwitch.vue';
import { date, files, jobStates, picks } from '../services/format';
import { errorMessage } from '../services/api';
afterEach(() => {
  vi.restoreAllMocks();
  setLocale('zh-CN');
});
describe('offline language catalogs', () => {
  it('updates enum labels reactively and interpolates counts', () => {
    const label = computed(() => picks.keep);
    expect(label.value).toBe('保留');
    setLocale('en-US');
    expect(label.value).toBe('Keep');
    expect(files.offline).toBe('Source offline');
    expect(jobStates.failed).toBe('Failed');
    expect(t('app.mediaCount', { count: 50000 })).toBe('50000 media files');
    expect(locale.value).toBe('en-US');
  });
  it('localizes error codes and retains precise native diagnostics', () => {
    setLocale('en-US');
    expect(errorText('SOURCE_OFFLINE')).toBe('Source offline. Reconnect it and try again.');
    expect(errorText('unknown')).toBe('The operation did not finish. Please try again.');
    expect(errorMessage({ code: 'DB_CONFLICT' })).toBe('The record changed. Refresh and try again.');
    expect(errorMessage({ code: 'DB_CONFLICT', message: 'Precise backend explanation' })).toBe(
      'Precise backend explanation',
    );
    expect(errorMessage({ code: 'DB_CONFLICT', message: '原生诊断：文件发生变化' })).toBe(
      'The record changed. Refresh and try again. · 原生诊断：文件发生变化',
    );
  });
  it('switches a mounted interface without resetting user input and remembers the choice', async () => {
    const wrapper = mount({
      components: { LanguageSwitch },
      setup: () => ({ tr, labels: translatedRecord({ settings: '设置' }) }),
      template:
        '<div><LanguageSwitch/><h1>{{ labels.settings }}</h1><input value="我的旅程.jpg"/><p>{{ tr("取消") }}</p></div>',
    });
    try {
      const input = wrapper.get('input').element;
      await wrapper.get('[lang="en"]').trigger('click');
      expect(wrapper.get('h1').text()).toBe('Settings');
      expect(wrapper.get('p').text()).toBe('Cancel');
      expect(wrapper.get('[lang="en"]').attributes('aria-pressed')).toBe('true');
      expect(wrapper.get('input').element).toBe(input);
      expect((input as HTMLInputElement).value).toBe('我的旅程.jpg');
      expect(localStorage.getItem('travel-gallery-language')).toBe('en-US');
      await wrapper.get('[lang="zh-CN"]').trigger('click');
      expect(wrapper.get('h1').text()).toBe('设置');
    } finally {
      wrapper.unmount();
    }
  });
  it('restores the saved language when the app module starts again', async () => {
    setLocale('en-US');
    vi.resetModules();
    const restarted = await import('../i18n');
    expect(restarted.locale.value).toBe('en-US');
  });
  it('still switches when browser storage is unavailable', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('Storage blocked');
    });
    expect(() => setLocale('en-US')).not.toThrow();
    expect(tr('设置')).toBe('Settings');
  });
  it('interpolates paths as data and preserves Chinese line breaks', () => {
    const name = '我的照片 {count}.jpg';
    setLocale('en-US');
    expect(tr('更新至 v{0}', { 0: name })).toBe(`Update to v${name}`);
    expect(tr('unmapped diagnostic')).toBe('unmapped diagnostic');
    expect(tr('__proto__')).toBe('__proto__');
    expect(tr('Unknown {constructor}')).toBe('Unknown {constructor}');
    setLocale('zh-CN');
    expect(tr('确认：\n{0}', { 0: name })).toBe(`确认：\n${name}`);
  });
  it('formats the Unix epoch as a known date rather than an unknown value', () => {
    expect(date(0)).not.toBe('—');
    expect(date(null)).toBe('—');
    expect(date(Number.NaN)).toBe('—');
  });
});
