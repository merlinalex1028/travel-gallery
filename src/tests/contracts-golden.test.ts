import { expect, it } from 'vitest';
import golden from '../../tests/fixtures/contracts.json';
import type { AppError, AssetDetail, AssetSummary, Commands, Page } from '../services/contracts';
function enumValue<T extends string>(value: string, allowed: readonly T[]): T {
  if (!allowed.includes(value as T)) throw new Error(`Invalid contract enum: ${value}`);
  return value as T;
}
function summary(raw: typeof golden.assetSummary): AssetSummary {
  return {
    ...raw,
    mediaKind: enumValue(raw.mediaKind, ['image', 'video']),
    fileState: enumValue(raw.fileState, ['available', 'offline', 'missing', 'unsupported', 'corrupt']),
    thumbnailState: enumValue(raw.thumbnailState, ['ready', 'queued', 'unsupported', 'failed']),
    review: {
      ...raw.review,
      pickState: enumValue(raw.review.pickState, ['unreviewed', 'keep', 'maybe', 'reject']),
    },
  };
}
it('accepts the Rust golden DTOs through the public TypeScript contracts', () => {
  const asset = summary(golden.assetSummary);
  const detail: AssetDetail = { ...golden.assetDetail, ...summary(golden.assetDetail) };
  const page: Page<AssetSummary> = { ...golden.page, items: golden.page.items.map(summary) };
  const neighbors: Commands['get_asset_neighbors']['result'] = {
    ...golden.neighbors,
    items: golden.neighbors.items.map(summary),
  };
  const progress: Commands['get_cull_progress']['result'] = golden.cullProgress;
  const review: Commands['apply_review_batch']['result'] = golden.reviewReceipt;
  const time: Commands['save_capture_time']['result'] = golden.captureTimeReceipt;
  const timeline: Commands['timeline_stats']['result'] = golden.timeline;
  const story: Commands['get_trip_story']['result'] = {
    ...golden.story,
    items: golden.story.items.map((item) => ({
      ...item,
      mediaKind: enumValue(item.mediaKind, ['image', 'video']),
      fileState: enumValue(item.fileState, ['available', 'offline', 'missing', 'unsupported', 'corrupt']),
    })),
  };
  const aiStatus: Commands['get_ai_status']['result'] = {
    ...golden.aiStatus,
    runtime: enumValue(golden.aiStatus.runtime, ['idle', 'loading', 'loaded']),
  };
  const aiClusters: Commands['get_ai_clusters']['result'] = golden.aiClusters;
  const qualityStatus: Commands['get_quality_status']['result'] = {
    ...golden.qualityStatus,
    runtime: enumValue(golden.qualityStatus.runtime, ['idle', 'loading', 'loaded']),
  };
  const qualityResults: Commands['get_quality_results']['result'] = golden.qualityResults;
  const assetQuality: Commands['get_asset_quality']['result'] = golden.assetQuality;
  expect(qualityStatus.eyesEnabled).toBe(false);
  expect(qualityStatus.detailEnabled).toBe(false);
  expect(qualityStatus.total).toBe(1);
  expect(qualityStatus.modelBytes).toBe(10005871);
  expect(qualityResults.items).toEqual([]);
  expect(assetQuality.observations).toEqual([]);
  expect(aiStatus.enabled).toBe(false);
  expect(aiStatus.runtime).toBe('idle');
  expect(aiStatus.total).toBe(1);
  expect(aiStatus.indexedCount).toBe(0);
  expect(aiClusters.groups).toEqual([]);
  expect(story.items[0]?.date).toBe('2026-06-15');
  expect(story.items[0]?.unknownTimezone).toBe(true);
  expect(story.trip.id).toBe('golden-trip');
  const exportReport: Commands['get_export_report']['result'] = {
    ...golden.exportReport,
    status: enumValue(golden.exportReport.status, ['completed']),
  };
  const preset: Commands['list_export_presets']['result'][number] = {
    ...golden.exportPreset,
    options: {
      ...golden.exportPreset.options,
      layout: enumValue(golden.exportPreset.options.layout, ['captureMonth']),
      conflictPolicy: enumValue(golden.exportPreset.options.conflictPolicy, ['skip']),
    },
  };
  const naming: Commands['get_rename_preview']['result'] = {
    ...golden.renamePreview,
    target: { ...golden.renamePreview.target, kind: 'snapshot' },
    job: { ...golden.renamePreview.job, type: 'REPAIR', operation: 'renamePreview', status: 'completed' },
    items: golden.renamePreview.items.map((item) => ({ ...item, status: enumValue(item.status, ['ready']) })),
  };
  expect(preset.options.naming?.start).toBe(7);
  expect(naming.items[0]?.proposedFilename).toBe('20260615_0007.JPG');
  const error: AppError = { ...golden.error, code: enumValue(golden.error.code, ['INVALID_FILTER']) };
  expect(page.items).toEqual([asset]);
  expect(neighbors.items).toEqual([asset]);
  expect(detail.id).toBe(asset.id);
  expect(detail.captureTimeCorrection?.revision).toBe(0);
  expect(detail.preview.canOriginalPreview).toBe(true);
  expect(progress.scope).toEqual({ reviewed: 0, total: 1 });
  expect(review.affectedCount).toBe(1);
  expect(time.revision).toBe(1);
  expect(timeline).toEqual([
    { date: '2026-06-15', count: 1, reviewed: 1, unknownTimezone: 1, photos: 1, videos: 0 },
  ]);
  expect(exportReport.copiedCount).toBe(1);
  expect(exportReport.failedCount).toBe(0);
  expect(exportReport.xmpFailedCount).toBe(1);
  expect(exportReport.sidecars[0]?.errorCode).toBe('EXPORT_CONFLICT');
  expect(error.retryable).toBe(false);
});
