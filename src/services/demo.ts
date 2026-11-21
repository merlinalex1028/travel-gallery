import eyeProfile from '../../src-tauri/resources/ai/face-landmarker-v1.json';
import eyeCard from '../../src-tauri/resources/ai/EYE_MODEL_CARD.md?raw';
import eyeLicense from '../../src-tauri/resources/ai/EYE_LICENSE.txt?raw';
import modelProfile from '../../src-tauri/resources/ai/siglip2-base224.json';
import modelCard from '../../src-tauri/resources/ai/MODEL_CARD.md?raw';
import modelLicense from '../../src-tauri/resources/ai/LICENSE.txt?raw';
import { settingsWithDefaults } from './settings';
import type {
  AppSettings,
  AssetDetail,
  AssetFilter,
  AssetSort,
  BurstGroup,
  Commands,
  DuplicateGroup,
  JobSummary,
  ReviewPatch,
  SavedFilter,
  ExportPreset,
  SelectionTarget,
  SourceRoot,
  Trip,
  TimelineDay,
} from './contracts';

// Browser-only, clearly labelled preview. Native builds never load this adapter.
const day = Date.UTC(2026, 5, 15);
const sourceId = 'demo-source';
const seed: AssetDetail[] = Array.from({ length: 30 }, (_, i) => ({
  id: `demo-${i + 1}`,
  sourceId,
  relativePath: `川西之旅/DSC_${String(1024 + i).padStart(4, '0')}.JPG`,
  filename: `DSC_${1024 + i}.JPG`,
  extension: i === 9 ? 'mov' : i === 16 ? 'arw' : 'jpg',
  mediaKind: i === 9 ? 'video' : 'image',
  fileState: 'available',
  byteSize: 3840240 + i * 34562,
  captureUtcMs: day + Math.floor(i / 10) * 86400000 + (i < 3 ? i * 1000 : i * 45000),
  captureLocal:
    i < 3
      ? `2026-06-15 09:20:0${i}`
      : `2026-06-${15 + Math.floor(i / 10)} 09:${String(20 + i).padStart(2, '0')}:00`,
  captureTimeSource: 'exif',
  captureTzMin: 480,
  width: i % 5 === 0 ? 4000 : 6000,
  height: 4000,
  durationMs: i === 9 ? 28000 : null,
  cameraModel: i >= 3 && i % 3 === 0 ? 'iPhone 16 Pro' : 'SONY α7 IV',
  cameraMake: i >= 3 && i % 3 === 0 ? 'Apple' : 'SONY',
  codec: null,
  thumbnailState: 'ready',
  indexedAt: day,
  mtimeMs: day,
  review: {
    pickState: i < 5 ? 'keep' : i === 5 ? 'maybe' : i === 6 ? 'reject' : 'unreviewed',
    rating: i < 5 ? 4 + (i % 2) : 0,
    isFavorite: [0, 2, 8, 15].includes(i),
    note: '',
    revision: 0,
    updatedAt: day,
  },
  absolutePath: `/演示/川西之旅/DSC_${1024 + i}.JPG`,
  gpsLat: 30.04 + (i % 3) * 0.02,
  gpsLon: 101.52 + (i % 4) * 0.05,
  orientation: 1,
  metadata: { 镜头: 'FE 24–70mm F2.8 GM II', 焦距: '35 mm', 光圈: 'f/2.8', 快门: '1/800 s', ISO: 100 },
  tagIds: i < 4 ? ['landscape'] : [],
  preview: {
    canThumbnail: true,
    canScreenPreview: true,
    canOriginalPreview: true,
    canPlayInline: false,
    canOpenSystem: false,
  },
}));
interface DemoState {
  exportPresets?: ExportPreset[];
  assets: AssetDetail[];
  tags: { id: string; name: string; color: string | null; count: number }[];
  trips: Trip[];
  tripAssets: Record<string, string[]>;
  settings: Record<string, unknown>;
  saved: SavedFilter[];
}
function load(): DemoState {
  try {
    const data = localStorage.getItem('travel-gallery-demo-v1');
    if (data) return JSON.parse(data) as DemoState;
  } catch {
    /* Fresh demo if storage is unavailable. */
  }
  return {
    assets: seed,
    tags: [
      { id: 'landscape', name: '风光', color: '#66846c', count: 4 },
      { id: 'moments', name: '旅途日常', color: '#b58d63', count: 0 },
    ],
    trips: [
      {
        id: 'west',
        name: '风从川西来',
        description: '山野、湖泊与路上的日常。',
        startAt: day,
        endAt: day + 86400000 * 3,
        coverAssetId: 'demo-1',
        count: 30,
      },
    ],
    tripAssets: { west: seed.map((a) => a.id) },
    settings: {},
    saved: [],
  };
}
let state = load();
let catalogEpoch = 0;
function save() {
  catalogEpoch++;
  localStorage.setItem('travel-gallery-demo-v1', JSON.stringify(state));
}
type DemoSnapshot = { ids: string[]; expiresAt: number };
function restoreSnapshots(): Array<[string, DemoSnapshot]> {
  try {
    return JSON.parse(sessionStorage.getItem('demo-snapshots-v1') ?? '[]') as Array<[string, DemoSnapshot]>;
  } catch {
    return [];
  }
}
const snapshots = new Map<string, DemoSnapshot>(restoreSnapshots()),
  contexts = new Map<string, { filter: AssetFilter; sort: AssetSort }>(),
  mutations = new Map<string, unknown>(),
  undo = new Map<string, AssetDetail[]>();
const jobs: JobSummary[] = [];
function snapshotIds(id: string) {
  const snapshot = snapshots.get(id);
  if (!snapshot || snapshot.expiresAt <= Date.now())
    throw { code: 'SNAPSHOT_EXPIRED', message: '冻结选区已过期，请重新选择并确认数量。' };
  return snapshot.ids;
}
function captureDate(asset: AssetDetail): string | null {
  const date = asset.captureLocal?.slice(0, 10);
  if (!date || !['exif', 'quicktime', 'manual'].includes(asset.captureTimeSource ?? '')) return null;
  const parsed = new Date(`${date}T00:00:00Z`);
  return Number.isFinite(parsed.getTime()) && parsed.toISOString().slice(0, 10) === date ? date : null;
}
function filterAssets(f: AssetFilter, sort: AssetSort) {
  const scopeIds = f.selectionSnapshotId ? new Set(snapshotIds(f.selectionSnapshotId)) : null;
  const result = state.assets.filter(
    (a) =>
      (!f.sourceIds?.length || f.sourceIds.includes(a.sourceId)) &&
      (!f.mediaKinds?.length || f.mediaKinds.includes(a.mediaKind)) &&
      (!f.extensions?.length || f.extensions.includes(a.extension)) &&
      (!f.pickStates?.length || f.pickStates.includes(a.review.pickState)) &&
      (f.ratingMin === undefined || a.review.rating >= f.ratingMin) &&
      (f.ratingMax === undefined || a.review.rating <= f.ratingMax) &&
      (f.favorite === undefined || a.review.isFavorite === f.favorite) &&
      (!f.fileStates?.length || f.fileStates.includes(a.fileState)) &&
      (!f.keyword ||
        [
          a.filename,
          a.relativePath,
          ...state.tags.filter((t) => a.tagIds.includes(t.id)).map((t) => t.name),
        ].some((s) => s.toLowerCase().includes(f.keyword!.toLowerCase()))) &&
      (!f.collectionId || state.tripAssets[f.collectionId]?.includes(a.id)) &&
      (!f.folder || a.relativePath.startsWith(f.folder)) &&
      (!f.captureFromMs || (a.captureUtcMs ?? 0) >= f.captureFromMs) &&
      (!f.captureToMs || (a.captureUtcMs ?? 0) <= f.captureToMs) &&
      (f.captureDateKnown === undefined || !!captureDate(a) === f.captureDateKnown) &&
      (!scopeIds || scopeIds.has(a.id)) &&
      (!f.captureLocalFrom || (a.captureLocal?.slice(0, 10) ?? '') >= f.captureLocalFrom) &&
      (!f.captureLocalTo || (a.captureLocal?.slice(0, 10) ?? '') <= f.captureLocalTo) &&
      (!f.gpsCell ||
        (Number(a.gpsLat?.toFixed(2)) === f.gpsCell[0] && Number(a.gpsLon?.toFixed(2)) === f.gpsCell[1])) &&
      (!f.minBytes || a.byteSize >= f.minBytes) &&
      (!f.maxBytes || a.byteSize <= f.maxBytes) &&
      (!f.tagRule?.tagIds.length ||
        (f.tagRule.mode === 'all'
          ? f.tagRule.tagIds.every((t) => a.tagIds.includes(t))
          : f.tagRule.mode === 'exclude'
            ? f.tagRule.tagIds.every((t) => !a.tagIds.includes(t))
            : f.tagRule.tagIds.some((t) => a.tagIds.includes(t)))),
  );
  return result.sort((a, b) => {
    const x = sort.field === 'rating' ? a.review.rating : (a[sort.field] ?? 0),
      y = sort.field === 'rating' ? b.review.rating : (b[sort.field] ?? 0);
    const d = x < y ? -1 : x > y ? 1 : a.id.localeCompare(b.id);
    return sort.direction === 'asc' ? d : -d;
  });
}
function target(t: SelectionTarget) {
  return t.kind === 'explicit'
    ? t.assetIds
    : snapshotIds(t.snapshotId).filter((i) => !t.excludeAssetIds?.includes(i));
}
function burstGroups(filter: AssetFilter, gap: number) {
  const candidates = filterAssets(filter, { field: 'captureUtcMs', direction: 'asc' }).filter(
    (a) =>
      a.mediaKind === 'image' &&
      a.fileState !== 'missing' &&
      a.captureLocal &&
      a.cameraModel &&
      ['exif', 'quicktime', 'manual'].includes(a.captureTimeSource ?? ''),
  );
  const partition = (a: AssetDetail) =>
    JSON.stringify([
      a.sourceId,
      a.relativePath.slice(0, -a.filename.length),
      a.cameraMake,
      a.cameraModel,
      a.captureTzMin,
      a.captureLocal?.slice(0, 10),
    ]);
  candidates.sort(
    (a, b) =>
      partition(a).localeCompare(partition(b)) ||
      a.captureLocal!.localeCompare(b.captureLocal!) ||
      a.id.localeCompare(b.id),
  );
  const groups: BurstGroup[] = [];
  let current: AssetDetail[] = [];
  const time = (a: AssetDetail) => Date.parse(a.captureLocal!.replace(' ', 'T') + 'Z');
  const finish = () => {
    if (current.length > 1) {
      const a = current[0]!;
      groups.push({
        id: current.map((a) => a.id).join(','),
        sourceId: a.sourceId,
        folder: a.relativePath.slice(0, -a.filename.length),
        cameraModel: a.cameraModel!,
        captureTzMin: a.captureTzMin,
        startLocal: a.captureLocal!,
        endLocal: current.at(-1)!.captureLocal!,
        memberCount: current.length,
        members: structuredClone(current),
      });
    }
  };
  for (const asset of candidates) {
    if (!Number.isFinite(time(asset))) continue;
    if (
      current.length &&
      (partition(current[0]!) !== partition(asset) ||
        time(asset) - time(current.at(-1)!) > gap * 1000 ||
        time(asset) - time(current[0]!) > 30000)
    ) {
      finish();
      current = [];
    }
    current.push(asset);
  }
  finish();
  groups.sort((a, b) => b.startLocal.localeCompare(a.startLocal) || a.id.localeCompare(b.id));
  return { groups, candidateCount: candidates.length };
}
function unsupported(): never {
  throw {
    code: 'PERMISSION_DENIED',
    message: '浏览器为演示模式。请运行桌面应用以访问本地文件、扫描、查重、导出和备份。',
  };
}
export async function demoCall(name: keyof Commands, request: unknown): Promise<unknown> {
  const r = (request ?? {}) as Record<string, unknown>;
  switch (name) {
    case 'get_eyes_model_info':
      return { profile: eyeProfile, card: eyeCard, licenseText: eyeLicense };
    case 'get_quality_status':
      return {
        eyesEnabled: false,
        detailEnabled: false,
        modelAvailable: false,
        modelFolderPresent: false,
        profileId: eyeProfile.id,
        profileName: eyeProfile.name,
        modelBytes: eyeProfile.files.reduce((n, f) => n + f.bytes, 0),
        installedAt: null,
        runtime: 'idle',
        inferenceThreads: 1,
        idleReleaseSeconds: 60,
        total: filterAssets((request as Commands['get_quality_status']['request']).filter, {
          field: 'filename',
          direction: 'asc',
        }).filter((a) => a.mediaKind === 'image').length,
        eyesCount: 0,
        detailCount: 0,
        eyeVersion: '',
        detailVersion: '',
      };
    case 'get_quality_results':
      return {
        items: [],
        total: 0,
        offset: (request as Commands['get_quality_results']['request']).offset,
        pageSize: 60,
      };
    case 'get_asset_quality':
      return { assetId: r.assetId, observations: [], staleCount: 0 };
    case 'set_detail_enabled':
    case 'set_eyes_enabled':
    case 'import_eyes_model':
    case 'remove_eyes_model':
    case 'start_quality_analysis':
    case 'set_quality_dismissed':
      return unsupported();
    case 'get_ai_model_info':
      return { profile: modelProfile, card: modelCard, licenseText: modelLicense };
    case 'get_ai_status': {
      const req = request as Commands['get_ai_status']['request'];
      return {
        enabled: false,
        modelAvailable: false,
        modelFolderPresent: false,
        modelVersion: `${modelProfile.id}/siglip2-screen-rgb-bilinear224-pad0-v1/candle-0.11.0`,
        profileId: modelProfile.id,
        profileName: modelProfile.name,
        modelBytes: modelProfile.files.reduce((sum, file) => sum + file.bytes, 0),
        installedAt: null,
        runtime: 'idle',
        inferenceThreads: 2,
        idleReleaseSeconds: 60,
        total: filterAssets(req.filter, { field: 'filename', direction: 'asc' }).length,
        indexedCount: 0,
      };
    }
    case 'get_ai_clusters':
      return {
        jobId: null,
        groups: [],
        summary: null,
        createdAt: null,
        filter: (request as Commands['get_ai_clusters']['request']).filter,
      };
    case 'list_export_presets':
      return state.exportPresets ?? [];
    case 'save_export_preset': {
      const req = request as Commands['save_export_preset']['request'];
      const presets = state.exportPresets ?? [];
      const prior = presets.find((p) => p.id === req.id);
      if (req.id && (!prior || prior.revision !== req.expectedRevision))
        throw new Error('预设已改变，请重新选择');
      const name = req.name.trim();
      if (
        !name ||
        name.length > 80 ||
        presets.some((p) => p.id !== req.id && p.name.toLowerCase() === name.toLowerCase())
      )
        throw new Error('预设名称无效或重名');
      const preset: ExportPreset = {
        id: prior?.id ?? crypto.randomUUID(),
        name,
        revision: (prior?.revision ?? 0) + 1,
        options: structuredClone(req.options),
      };
      state.exportPresets = [...presets.filter((p) => p.id !== preset.id), preset];
      save();
      return preset;
    }
    case 'delete_export_preset': {
      const req = request as Commands['delete_export_preset']['request'];
      const prior = state.exportPresets?.find((p) => p.id === req.id);
      if (!prior || prior.revision !== req.expectedRevision) throw new Error('预设已改变，请重新选择');
      state.exportPresets = state.exportPresets!.filter((p) => p.id !== req.id);
      save();
      return { updated: true };
    }
    case 'preview_rename':
    case 'get_rename_preview':
      return unsupported();
    case 'get_app_status':
      return {
        databaseReady: true,
        catalogVersion: 6,
        catalogEpoch: 0,
        power: { onBattery: null },
        offlineMode: true,
        watcher: { active: false, watchedRoots: 0, pollingRoots: 0, lastError: null },
      };
    case 'list_sources':
      return [
        {
          id: sourceId,
          displayName: '川西之旅 · 演示',
          absolutePath: '/演示资料库/川西之旅',
          volumeId: null,
          isEnabled: true,
          isOnline: true,
          lastScanAt: day,
          totalAssets: state.assets.length,
          offlineAssets: 0,
        },
      ] satisfies SourceRoot[];
    case 'query_assets': {
      const req = request as Commands['query_assets']['request'];
      const list = filterAssets(req.filter, req.sort),
        fp = JSON.stringify([req.filter, req.sort]);
      contexts.set(fp, { filter: req.filter, sort: req.sort });
      const offset = Number(req.cursor ?? 0);
      const items = structuredClone(list.slice(offset, offset + req.pageSize));
      const frozen = req.selectionSnapshotId ? new Set(snapshotIds(req.selectionSnapshotId)) : null;
      return {
        items,
        total: list.length,
        nextCursor: offset + req.pageSize < list.length ? String(offset + req.pageSize) : null,
        queryFingerprint: fp,
        catalogEpoch,
        ...(frozen
          ? {
              selectionMembership: {
                snapshotId: req.selectionSnapshotId!,
                assetIds: items.filter((a) => frozen.has(a.id)).map((a) => a.id),
              },
            }
          : {}),
      };
    }
    case 'get_asset': {
      const a = state.assets.find((a) => a.id === r.assetId);
      if (!a) throw { message: '媒体不存在' };
      return structuredClone(a);
    }
    case 'save_capture_time': {
      const req = request as Commands['save_capture_time']['request'];
      const a = state.assets.find((a) => a.id === req.assetId);
      if (!a) throw { message: '媒体不存在' };
      const previous = a.captureTimeCorrection;
      if ((previous?.revision ?? 0) !== req.expectedRevision)
        throw { message: '拍摄时间已更新，请重新打开详情' };
      const original = previous?.original ?? {
        captureLocal: a.captureLocal,
        captureUtcMs: a.captureUtcMs,
        captureTzMin: a.captureTzMin,
        captureTimeSource: a.captureTimeSource,
      };
      a.captureTimeCorrection = {
        original,
        isCorrected: req.captureLocal !== null,
        revision: (previous?.revision ?? 0) + 1,
      };
      if (req.captureLocal === null) Object.assign(a, original);
      else {
        a.captureLocal = req.captureLocal;
        a.captureTzMin = req.captureTzMin;
        a.captureUtcMs =
          req.captureTzMin === null
            ? null
            : Date.parse(req.captureLocal.replace(' ', 'T') + 'Z') - req.captureTzMin * 60000;
        a.captureTimeSource = 'manual';
      }
      save();
      return { updated: true, revision: a.captureTimeCorrection.revision };
    }
    case 'map_trails': {
      const req = request as Commands['map_trails']['request'];
      const photos = filterAssets(req.filter, { field: 'captureUtcMs', direction: 'asc' }).filter(
        (a) => a.gpsLat !== null && a.gpsLon !== null && a.captureLocal && a.captureTimeSource !== 'mtime',
      );
      const groups = new Map<string, Array<[number, number]>>();
      for (const a of photos.slice(0, 5000)) {
        const date = a.captureLocal!.slice(0, 10);
        const line = groups.get(date) ?? [];
        const point: [number, number] = [
          Math.round(a.gpsLon! * 100) / 100,
          Math.round(a.gpsLat! * 100) / 100,
        ];
        if (line.at(-1)?.join(',') !== point.join(',')) line.push(point);
        groups.set(date, line);
      }
      return {
        type: 'FeatureCollection',
        features: [...groups]
          .filter(([, line]) => line.length > 1)
          .map(([date, coordinates]) => ({
            type: 'Feature',
            properties: { date, unknownTimezone: false },
            geometry: { type: 'LineString', coordinates },
          })),
        totalPhotos: photos.length,
        includedPhotos: Math.min(5000, photos.length),
        truncated: photos.length > 5000,
        unknownTimezone: 0,
        skippedInvalidTimes: 0,
      };
    }
    case 'get_cull_progress': {
      const req = request as Commands['get_cull_progress']['request'];
      const anchor = state.assets.find((a) => a.id === req.assetId);
      if (!anchor) throw { message: '媒体不存在' };
      const list = filterAssets(
        { ...req.filter, pickStates: undefined },
        { field: 'filename', direction: 'asc' },
      );
      const date = anchor.captureLocal?.slice(0, 10) ?? '未知日期';
      const parent = anchor.relativePath.slice(0, -anchor.filename.length);
      const count = (items: AssetDetail[]) => ({
        total: items.length,
        reviewed: items.filter((a) => a.review.pickState !== 'unreviewed').length,
      });
      return {
        scope: count(list),
        day: { label: date, ...count(list.filter((a) => a.captureLocal?.slice(0, 10) === date)) },
        folder: {
          label: parent.replace(/\/$/, '') || '来源根目录',
          ...count(
            list.filter(
              (a) => a.sourceId === anchor.sourceId && a.relativePath.slice(0, -a.filename.length) === parent,
            ),
          ),
        },
      };
    }
    case 'resolve_original':
      return {
        resourceUrl: `/demo/landscape-${(Number(String(r.assetId).split('-').at(-1)) - 1) % 12}.svg`,
        mimeType: 'image/svg+xml',
      };
    case 'resolve_preview':
      return {
        assetId: r.assetId,
        variant: r.variant,
        resourceUrl: `/demo/landscape-${(Number(String(r.assetId).split('-').at(-1)) - 1) % 12}.svg`,
        ready: true,
        mimeType: 'image/svg+xml',
      };
    case 'get_asset_neighbors': {
      const ctx = contexts.get(String(r.queryFingerprint));
      const list = ctx ? filterAssets(ctx.filter, ctx.sort) : state.assets;
      const i = list.findIndex((a) => a.id === r.assetId);
      const limit = Math.max(1, Math.min(10, Number(r.limit) || 1));
      const memberIds = new Set(list.map((a) => a.id));
      const window =
        i >= 0
          ? list
          : filterAssets({}, ctx?.sort ?? { field: 'captureUtcMs', direction: 'desc' }).filter(
              (a) => a.id === r.assetId || memberIds.has(a.id),
            );
      const anchor = window.findIndex((a) => a.id === r.assetId);
      return {
        previousId: window[anchor - 1]?.id ?? null,
        nextId: window[anchor + 1]?.id ?? null,
        items: window.slice(Math.max(0, anchor - limit), anchor + limit + 1),
        anchorMatches: i >= 0,
        total: list.length,
        position: i >= 0 ? i + 1 : null,
      };
    }
    case 'create_selection_snapshot': {
      const req = request as Commands['create_selection_snapshot']['request'];
      if (req.expectedCatalogEpoch !== undefined && req.expectedCatalogEpoch !== catalogEpoch)
        throw { code: 'INVALID_FILTER', message: '资料库已变化，请刷新结果后重新选择' };
      const ids = req.target ? new Set(target(req.target)) : null;
      const list = filterAssets(req.filter, req.sort).filter((a) => !ids || ids.has(a.id)),
        id = crypto.randomUUID();
      snapshots.set(id, { ids: list.map((a) => a.id), expiresAt: Date.now() + 86400000 });
      sessionStorage.setItem('demo-snapshots-v1', JSON.stringify([...snapshots]));
      return {
        id,
        count: list.length,
        createdAt: Date.now(),
        expiresAt: Date.now() + 86400000,
        queryFingerprint: JSON.stringify([req.filter, req.sort]),
      };
    }
    case 'apply_review_batch':
    case 'apply_tags_batch': {
      const key = String(r.clientMutationId);
      if (mutations.has(key)) return mutations.get(key);
      const ids = target(r.target as SelectionTarget),
        id = crypto.randomUUID();
      undo.set(id, structuredClone(state.assets.filter((a) => ids.includes(a.id))));
      let n = 0;
      for (const a of state.assets.filter((a) => ids.includes(a.id))) {
        if (name === 'apply_review_batch') Object.assign(a.review, r.patch as ReviewPatch);
        else
          a.tagIds = [
            ...new Set([
              ...a.tagIds.filter((t) => !(r.removeTagIds as string[]).includes(t)),
              ...(r.addTagIds as string[]),
            ]),
          ];
        a.review.revision++;
        n++;
      }
      save();
      const result = { changeBatchId: id, affectedCount: n, unchangedCount: 0, conflicts: [] };
      mutations.set(key, result);
      return result;
    }
    case 'undo_review_batch': {
      const before = undo.get(String(r.changeBatchId)) ?? [];
      let n = 0;
      for (const a of before) {
        const current = state.assets.find((b) => b.id === a.id);
        if (current?.review.revision === a.review.revision + 1) {
          Object.assign(current, a);
          current.review.revision += 2;
          n++;
        }
      }
      undo.delete(String(r.changeBatchId));
      save();
      return { changeBatchId: r.changeBatchId, restoredCount: n, skippedCount: before.length - n };
    }
    case 'list_tags':
      return state.tags.map((t) => ({
        ...t,
        count: state.assets.filter((a) => a.tagIds.includes(t.id)).length,
      }));
    case 'upsert_tag': {
      const name = String(r.name).trim();
      if (!name || state.tags.some((t) => t.name.toLowerCase() === name.toLowerCase() && t.id !== r.id))
        throw { message: '标签名称为空或已存在' };
      const id = r.id ? String(r.id) : crypto.randomUUID(),
        old = state.tags.find((t) => t.id === id);
      if (old) Object.assign(old, { name, color: r.color });
      else state.tags.push({ id, name, color: r.color as string | null, count: 0 });
      save();
      return { tagId: id };
    }
    case 'delete_tag':
      state.tags = state.tags.filter((t) => t.id !== r.tagId);
      state.assets.forEach((a) => {
        a.tagIds = a.tagIds.filter((t) => t !== r.tagId);
      });
      save();
      return { updated: true };
    case 'merge_tags': {
      let n = 0;
      for (const a of state.assets)
        if (a.tagIds.includes(String(r.sourceTagId))) {
          a.tagIds = [...new Set([...a.tagIds.filter((t) => t !== r.sourceTagId), String(r.targetTagId)])];
          n++;
        }
      state.tags = state.tags.filter((t) => t.id !== r.sourceTagId);
      save();
      return { affectedCount: n };
    }
    case 'list_jobs':
      return { items: jobs, total: jobs.length, nextCursor: null, queryFingerprint: 'jobs' };
    case 'query_duplicate_groups': {
      const kind = r.kind as 'exact' | 'similar';
      const members = state.assets.slice(kind === 'exact' ? 10 : 0, kind === 'exact' ? 12 : 3);
      const group: DuplicateGroup = {
        id: `demo-${kind}`,
        kind,
        algorithm: kind === 'exact' ? 'SHA-256' : 'dHash-64',
        algorithmVersion: '1',
        memberCount: members.length,
        representativeAssetId: null,
        members: members.map((a, i) => ({
          asset: structuredClone(a),
          distance: kind === 'exact' ? null : i * 2,
          excluded: false,
        })),
      };
      return { items: [group], total: 1, nextCursor: null, queryFingerprint: kind };
    }
    case 'query_burst_groups': {
      const req = request as Commands['query_burst_groups']['request'];
      const { groups, candidateCount } = burstGroups(req.filter, req.gapSeconds);
      const offset = Number(req.cursor ?? 0),
        end = offset + req.pageSize;
      return {
        items: groups.slice(offset, end).map((g) => ({ ...g, members: g.members.slice(0, 8) })),
        total: groups.length,
        nextCursor: end < groups.length ? String(end) : null,
        queryFingerprint: 'demo-bursts',
        candidateCount,
        examinedCount: candidateCount,
        skippedInvalidTimes: 0,
        truncated: false,
      };
    }
    case 'create_burst_snapshot': {
      const req = request as Commands['create_burst_snapshot']['request'];
      const group = burstGroups(req.filter, req.gapSeconds).groups.find((g) => g.id === req.groupId);
      if (!group) throw { code: 'DB_CONFLICT', message: '拍摄组已变化，请刷新后重新选择' };
      const id = crypto.randomUUID(),
        at = Date.now();
      snapshots.set(id, { ids: group.members.map((a) => a.id), expiresAt: at + 86400000 });
      sessionStorage.setItem('demo-snapshots-v1', JSON.stringify([...snapshots]));
      return {
        id,
        count: group.memberCount,
        createdAt: at,
        expiresAt: at + 86400000,
        queryFingerprint: 'demo-bursts',
      };
    }
    case 'get_settings':
      return settingsWithDefaults(state.settings as Partial<AppSettings>);
    case 'save_settings':
      state.settings = r;
      save();
      return { updated: true };
    case 'get_session':
      return JSON.parse(sessionStorage.getItem('demo-session') ?? 'null') as unknown;
    case 'save_session':
      sessionStorage.setItem('demo-session', JSON.stringify(r));
      return { updated: true };
    case 'cache_stats':
      return { bytes: 0, count: state.assets.length };
    case 'clear_cache':
      return { updated: true };
    case 'get_trip_story': {
      const req = request as Commands['get_trip_story']['request'];
      const trip = state.trips.find((t) => t.id === req.tripId);
      if (!trip) throw { code: 'INVALID_SELECTION', message: '旅行相册不存在' };
      const members = new Set(state.tripAssets[req.tripId] ?? []);
      const items = state.assets
        .filter(
          (asset) => members.has(asset.id) && (req.range === 'all' || asset.review.pickState === 'keep'),
        )
        .map((asset) => {
          const date = captureDate(asset);
          return {
            assetId: asset.id,
            filename: asset.filename,
            mediaKind: asset.mediaKind,
            fileState: asset.fileState,
            date,
            unknownTimezone: !!date && asset.captureTzMin == null,
            note: asset.review.note,
            capture: date ? asset.captureLocal : null,
          };
        })
        .sort(
          (a, b) =>
            (a.date ?? '9999').localeCompare(b.date ?? '9999') ||
            (a.capture ?? '').localeCompare(b.capture ?? '') ||
            a.assetId.localeCompare(b.assetId),
        )
        .map(({ capture: _capture, ...item }) => item);
      const epoch = catalogEpoch;
      if (req.epoch !== null && req.epoch !== epoch)
        throw { code: 'DB_CONFLICT', message: '旅行内容已变化，请刷新故事' };
      const end = req.offset + req.pageSize;
      return {
        trip,
        items: items.slice(req.offset, end),
        total: items.length,
        epoch,
        nextOffset: end < items.length ? end : null,
      };
    }
    case 'list_trips':
      return state.trips;
    case 'save_trip': {
      const req = request as Commands['save_trip']['request'],
        id = req.id ?? crypto.randomUUID(),
        old = state.trips.find((t) => t.id === id);
      if (old) Object.assign(old, req);
      else {
        state.trips.push({ ...req, id, coverAssetId: null, count: 0 });
        state.tripAssets[id] = [];
      }
      save();
      return { tripId: id };
    }
    case 'delete_trip':
      state.trips = state.trips.filter((t) => t.id !== r.tripId);
      save();
      return { updated: true };
    case 'add_to_trip':
    case 'remove_from_trip': {
      const tid = String(r.tripId),
        ids = target(r.target as SelectionTarget);
      const old = state.tripAssets[tid] ?? [];
      state.tripAssets[tid] =
        name === 'add_to_trip' ? [...new Set([...old, ...ids])] : old.filter((i) => !ids.includes(i));
      const trip = state.trips.find((t) => t.id === tid);
      if (trip) trip.count = state.tripAssets[tid].length;
      save();
      return { updated: true };
    }
    case 'set_trip_cover': {
      const t = state.trips.find((t) => t.id === r.tripId);
      if (t) t.coverAssetId = String(r.assetId);
      save();
      return { updated: true };
    }
    case 'list_saved_filters':
      return state.saved;
    case 'save_filter': {
      const req = request as Commands['save_filter']['request'];
      state.saved.push({ ...req, id: crypto.randomUUID() });
      save();
      return { updated: true };
    }
    case 'delete_filter':
      state.saved = state.saved.filter((f) => f.id !== r.filterId);
      save();
      return { updated: true };
    case 'get_media_links':
      return [];
    case 'timeline_stats': {
      const groups = new Map<string | null, TimelineDay>();
      for (const asset of filterAssets((r.filter ?? {}) as AssetFilter, {
        field: 'filename',
        direction: 'asc',
      })) {
        const date = captureDate(asset);
        const group = groups.get(date) ?? {
          date,
          count: 0,
          reviewed: 0,
          unknownTimezone: 0,
          photos: 0,
          videos: 0,
        };
        group.count++;
        if (asset.review.pickState !== 'unreviewed') group.reviewed++;
        if (date && asset.captureTzMin === null) group.unknownTimezone++;
        if (asset.mediaKind === 'image') group.photos++;
        else group.videos++;
        groups.set(date, group);
      }
      return [...groups.values()].sort((a, b) =>
        a.date === null ? (b.date === null ? 0 : 1) : b.date === null ? -1 : b.date.localeCompare(a.date),
      );
    }
    case 'map_assets': {
      const groups = new Map<string, { lat: number; lon: number; count: number; assetId: string }>();
      for (const asset of filterAssets((r.filter ?? {}) as AssetFilter, {
        field: 'filename',
        direction: 'asc',
      })) {
        if (asset.gpsLat === null || asset.gpsLon === null) continue;
        const lat = Number(asset.gpsLat.toFixed(2)),
          lon = Number(asset.gpsLon.toFixed(2)),
          key = `${lat},${lon}`;
        const point = groups.get(key) ?? { lat, lon, count: 0, assetId: asset.id };
        point.count++;
        groups.set(key, point);
      }
      return [...groups.values()];
    }
    default:
      return unsupported();
  }
}
