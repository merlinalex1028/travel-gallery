import { afterEach, describe, expect, it } from 'vitest';
import { computed } from 'vue';
import { errorText, locale, setLocale, t } from '../i18n';
import { date, files, jobStates, picks } from '../services/format';
import { errorMessage } from '../services/api';
afterEach(() => setLocale('zh-CN'));
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
  it('uses Chinese fallback for missing translations and unknown error codes', () => {
    setLocale('en-US');
    expect(errorText('SOURCE_OFFLINE')).toBe('来源离线，请连接后重试');
    expect(errorText('unknown')).toBe('The operation did not finish. Please try again.');
    expect(errorMessage({ code: 'DB_CONFLICT' })).toBe('The record changed. Refresh and try again.');
    expect(errorMessage({ code: 'DB_CONFLICT', message: 'Precise backend explanation' })).toBe(
      'Precise backend explanation',
    );
  });
  it('formats the Unix epoch as a known date rather than an unknown value', () => {
    expect(date(0)).not.toBe('—');
    expect(date(null)).toBe('—');
    expect(date(Number.NaN)).toBe('—');
  });
});
