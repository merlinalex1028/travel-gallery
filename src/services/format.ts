import type { FileState, PickState, JobStatus, JobType } from './contracts';
import { locale, localizedRecord } from '../i18n';
export const picks = localizedRecord<PickState>({
  unreviewed: 'review.unreviewed',
  keep: 'review.keep',
  maybe: 'review.maybe',
  reject: 'review.reject',
});
export const files = localizedRecord<FileState>({
  available: 'file.available',
  offline: 'file.offline',
  missing: 'file.missing',
  unsupported: 'file.unsupported',
  corrupt: 'file.corrupt',
});
export const jobStates = localizedRecord<JobStatus>({
  queued: 'job.queued',
  running: 'job.running',
  paused: 'job.paused',
  completed: 'job.completed',
  failed: 'job.failed',
  cancelled: 'job.cancelled',
});
export const jobTypes = localizedRecord<JobType>({
  SCAN: 'task.SCAN',
  METADATA: 'task.METADATA',
  THUMBNAIL: 'task.THUMBNAIL',
  HASH_EXACT: 'task.HASH_EXACT',
  HASH_SIMILAR: 'task.HASH_SIMILAR',
  EXPORT: 'task.EXPORT',
  BACKUP: 'task.BACKUP',
  REPAIR: 'task.REPAIR',
});
export function bytes(n: number) {
  if (n < 1024) return `${n} B`;
  const i = Math.min(3, Math.floor(Math.log(n) / Math.log(1024)));
  return `${(n / 1024 ** i).toFixed(i > 1 ? 1 : 0)} ${['B', 'KB', 'MB', 'GB'][i]}`;
}
export function date(n: number | null | undefined, full = false) {
  return n !== null && n !== undefined && Number.isFinite(n)
    ? new Intl.DateTimeFormat(locale.value, {
        year: 'numeric',
        month: '2-digit',
        day: '2-digit',
        ...(full ? ({ hour: '2-digit', minute: '2-digit' } as const) : {}),
      }).format(n)
    : '—';
}
export function duration(ms: number | null) {
  const s = Math.floor((ms ?? 0) / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
