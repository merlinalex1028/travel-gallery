/** Travel Gallery v1.1 public frontend/Rust DTO contract.
 * Types/spec only: the backend implementation must serialize via serde camelCase.
 * All time values are Unix milliseconds; null means unknown.
 */
export type Id = string;
export type PickState = 'unreviewed' | 'keep' | 'maybe' | 'reject';
export type FileState = 'available' | 'offline' | 'missing' | 'unsupported' | 'corrupt';
export type MediaKind = 'image' | 'video';
export type JobStatus = 'queued' | 'running' | 'paused' | 'completed' | 'failed' | 'cancelled';
export type JobType =
  'SCAN' | 'METADATA' | 'THUMBNAIL' | 'HASH_EXACT' | 'HASH_SIMILAR' | 'EXPORT' | 'BACKUP' | 'REPAIR';

export interface AppError {
  code:
    | 'SOURCE_EXISTS'
    | 'SOURCE_OVERLAP'
    | 'SOURCE_OFFLINE'
    | 'SOURCE_ACCESS_DENIED'
    | 'SOURCE_PATH_INVALID'
    | 'ASSET_NOT_FOUND'
    | 'ASSET_UNSUPPORTED'
    | 'ASSET_CORRUPT'
    | 'DB_CONFLICT'
    | 'DB_MIGRATION_FAILED'
    | 'SNAPSHOT_EXPIRED'
    | 'INVALID_FILTER'
    | 'INVALID_SELECTION'
    | 'JOB_NOT_CANCELLABLE'
    | 'EXPORT_CONFLICT'
    | 'EXPORT_NO_SPACE'
    | 'EXPORT_TARGET_UNSAFE'
    | 'PERMISSION_DENIED'
    | 'AI_DISABLED'
    | 'AI_MODEL_MISSING'
    | 'AI_MODEL_INVALID'
    | 'AI_RUNTIME_FAILED'
    | 'AI_INFERENCE_FAILED'
    | 'CANCELLED'
    | 'INTERNAL';
  message: string;
  retryable: boolean;
  details?: Record<string, unknown>;
}

export interface SourceRoot {
  id: Id;
  displayName: string;
  absolutePath: string;
  volumeId: string | null;
  isEnabled: boolean;
  isOnline: boolean;
  lastScanAt: number | null;
  totalAssets: number;
  offlineAssets: number;
}
export interface SourceOptions {
  recursive: boolean;
  skipHidden: boolean;
  excludeGlobs: string[];
  followSymlinks: false; // P0 prohibits symlink following for safety
  maxDepth?: number;
}
export interface AssetReview {
  pickState: PickState;
  rating: number;
  isFavorite: boolean;
  note: string;
  revision: number;
  updatedAt: number;
}
export interface AssetSummary {
  id: Id;
  sourceId: Id;
  relativePath: string;
  filename: string;
  extension: string;
  mediaKind: MediaKind;
  fileState: FileState;
  byteSize: number;
  captureUtcMs: number | null;
  captureLocal: string | null;
  width: number | null;
  height: number | null;
  durationMs: number | null;
  cameraModel: string | null;
  thumbnailState: 'ready' | 'queued' | 'unsupported' | 'failed';
  review: AssetReview;
  indexedAt?: number;
  mtimeMs?: number | null;
  linkedMediaCount?: number;
  linkedMediaType?: 'live_photo' | 'raw_jpeg' | null;
}
export interface AssetDetail extends AssetSummary {
  absolutePath: string; // display only; never use as backend file-write authorization
  cameraMake: string | null;
  codec: string | null;
  gpsLat: number | null;
  gpsLon: number | null;
  orientation: number | null;
  captureTzMin: number | null;
  captureTimeSource: string | null;
  captureTimeCorrection?: {
    original: {
      captureLocal: string | null;
      captureUtcMs: number | null;
      captureTzMin: number | null;
      captureTimeSource: string | null;
    } | null;
    isCorrected: boolean;
    revision: number;
  };
  metadata: Record<string, string | number | boolean | null>;
  preview: PreviewCapability;
  tagIds: Id[];
}
export interface PreviewCapability {
  canThumbnail: boolean;
  canScreenPreview: boolean;
  canOriginalPreview: boolean;
  canPlayInline: boolean;
  canOpenSystem: boolean;
  reason?: string;
}
export type AssetSortField = 'captureUtcMs' | 'indexedAt' | 'filename' | 'byteSize' | 'rating';
export interface AssetSort {
  field: AssetSortField;
  direction: 'asc' | 'desc';
}
export interface AssetFilter {
  sourceIds?: Id[];
  mediaKinds?: MediaKind[];
  extensions?: string[];
  pickStates?: PickState[];
  ratingMin?: number;
  ratingMax?: number;
  favorite?: boolean;
  fileStates?: FileState[];
  captureFromMs?: number;
  captureToMs?: number;
  /** Calendar dates compare the media's original wall time, including unknown timezone EXIF. */
  captureLocalFrom?: string;
  captureLocalTo?: string;
  /** Reliable original/manual wall date; false includes missing dates and mtime-only estimates. */
  captureDateKnown?: boolean;
  gpsCell?: [number, number];
  combineLinked?: boolean;
  minBytes?: number;
  maxBytes?: number;
  minWidth?: number;
  minHeight?: number;
  cameraModels?: string[];
  tagRule?: { mode: 'any' | 'all' | 'exclude'; tagIds: Id[] };
  duplicateKind?: 'exact' | 'similar' | 'any' | 'none';
  keyword?: string;
  folder?: string;
  indexedFromMs?: number;
  indexedToMs?: number;
  durationMinMs?: number;
  durationMaxMs?: number;
  collectionId?: string;
  aiClusterId?: string;
  selectionSnapshotId?: string;
}
export interface QueryAssetsRequest {
  filter: AssetFilter;
  sort: AssetSort;
  cursor: string | null;
  pageSize: number;
  /** Annotate current-page membership without restricting the query to this snapshot. */
  selectionSnapshotId?: Id;
}
export interface Page<T> {
  items: T[];
  nextCursor: string | null;
  total: number;
  queryFingerprint: string;
}
export interface QueryAssetsResult extends Page<AssetSummary> {
  selectionMembership?: { snapshotId: Id; assetIds: Id[] };
}
export type SelectionTarget =
  { kind: 'explicit'; assetIds: Id[] } | { kind: 'snapshot'; snapshotId: Id; excludeAssetIds?: Id[] };
export interface CreateSelectionSnapshotRequest {
  filter: AssetFilter;
  sort: AssetSort;
  target?: SelectionTarget;
}
export interface SelectionSnapshot {
  id: Id;
  count: number;
  createdAt: number;
  expiresAt: number;
  queryFingerprint: string;
}
export interface ReviewPatch {
  pickState?: PickState;
  rating?: number;
  isFavorite?: boolean;
  note?: string;
}
export interface ApplyReviewBatchRequest {
  target: SelectionTarget;
  patch: ReviewPatch;
  clientMutationId: Id; // idempotency key for retries
  expectedRevisions?: Record<Id, number>; // explicit selection only
}
export interface BatchApplyResult {
  changeBatchId: Id;
  affectedCount: number;
  unchangedCount: number;
  conflicts: { assetId: Id; actualRevision: number }[];
}
export interface UndoBatchResult {
  changeBatchId: Id;
  restoredCount: number;
  skippedCount: number;
}
export interface JobSummary {
  operation?:
    | 'renamePreview'
    | 'storyExport'
    | 'aiImportModel'
    | 'aiRemoveModel'
    | 'aiIndex'
    | 'aiCluster'
    | 'eyesImportModel'
    | 'eyesRemoveModel'
    | 'qualityAnalyze'
    | null;
  autoPauseReason?: 'battery' | null;
  cancelRequested?: boolean;
  id: Id;
  type: JobType;
  status: JobStatus;
  progressDone: number;
  progressTotal: number | null;
  failureCount: number;
  errorSummary: string | null;
  createdAt: number;
  updatedAt: number;
}
export type JobEvent =
  | { type: 'started'; jobId: Id; at: number }
  | { type: 'progress'; jobId: Id; at: number; done: number; total: number | null; failures: number }
  | { type: 'finished'; jobId: Id; at: number; status: 'completed' | 'failed' | 'cancelled' };
export interface DuplicateScanRequest {
  scope: SelectionTarget;
  kind: 'exact' | 'similar';
  similarity?: { maxHammingDistance: number; timeWindowSeconds: number; algorithmVersion: string };
}
export interface DuplicateGroup {
  id: Id;
  kind: 'exact' | 'similar';
  algorithm: string;
  algorithmVersion: string;
  memberCount: number;
  representativeAssetId: Id | null;
  members: Array<{ asset: AssetSummary; distance: number | null; excluded: boolean }>;
  stale?: boolean;
  totalMembers?: number;
}
export interface BurstGroup {
  id: Id;
  sourceId: Id;
  folder: string;
  cameraModel: string;
  captureTzMin: number | null;
  startLocal: string;
  endLocal: string;
  memberCount: number;
  members: AssetSummary[];
}
export type ExportConflictPolicy = 'skip' | 'rename'; // overwrite prohibited in P0
export interface NamingRule {
  pattern: string;
  start: number;
  padding: number;
}
export interface ExportOptions {
  layout: 'flat' | 'captureMonth' | 'sourceRelative';
  conflictPolicy: ExportConflictPolicy;
  includeLinked: boolean;
  includeXmp: boolean;
  naming?: NamingRule | null;
}
export interface ExportPreset {
  id: Id;
  name: string;
  revision: number;
  options: ExportOptions;
}
export interface ExportPlanRequest extends Omit<ExportOptions, 'includeLinked' | 'includeXmp'> {
  includeLinked?: boolean;
  includeXmp?: boolean;
  target: SelectionTarget;
  destinationPath: string; // must match a recently user-authorized directory
}
export interface ExportPreview {
  examples?: { assetId: Id; sourceRelativePath: string; destinationRelativePath: string; skipped: boolean }[];
  examplesTruncated?: boolean;
  planToken: string;
  expiresAt: number;
  fileCount: number;
  xmpCount: number;
  totalBytes: number;
  conflictCount: number;
  offlineCount: number;
  duplicateDestinationCount: number;
  warnings: string[];
}
export interface ExportStartRequest {
  planToken: string;
  clientMutationId: Id;
}
export interface RenamePreview {
  job: JobSummary;
  rule: NamingRule;
  target: SelectionTarget;
  summary: {
    total: number;
    ready: number;
    unchanged: number;
    conflict: number;
    invalid: number;
    unavailable: number;
  } | null;
  items: {
    assetId: Id;
    sourceId: Id;
    sourceName: string;
    sourceRelativePath: string;
    originalFilename: string;
    proposedFilename: string | null;
    proposedRelativePath: string | null;
    status: 'ready' | 'unchanged' | 'conflict' | 'invalid' | 'unavailable';
    reason: string;
    dateUnknown: boolean;
  }[];
  nextOffset: number | null;
  expiresAt: number | null;
}
export interface StoryExportResult {
  folder: string;
  includeNotes: boolean;
  result: {
    folder: string;
    html: { relative: string; bytes: number; sha256: string } | null;
    manifest: { relative: string; bytes: number; sha256: string } | null;
  } | null;
}
export interface ExportReport {
  story?: StoryExportResult;
  jobId: Id;
  status: JobStatus;
  copiedCount: number;
  skippedCount: number;
  failedCount: number;
  verifiedCount: number;
  manifestPath: string | null;
  xmpCopiedCount: number;
  xmpSkippedCount: number;
  xmpFailedCount: number;
  sidecars: ExportReportItem[];
}
export interface PreviewHandle {
  assetId: Id;
  variant: 'small' | 'medium' | 'screen' | 'poster';
  resourceUrl: string | null; // backend-authorized local asset token only
  mimeType: string | null;
  ready: boolean;
  reason?: string;
}
export interface RelinkPreview {
  sourceId: Id;
  newPath: string;
  matched: number;
  conflicts: number;
  missing: number;
  sampleChecks: Array<{ relativePath: string; matched: boolean; reason?: string }>;
  verificationToken: string;
}

export interface AppSettings {
  pauseOnBattery: boolean;
  theme: 'system' | 'light' | 'dark';
  density: number;
  autoAdvance: boolean;
  scanConcurrency: number;
  cacheLimitGb: number;
  showInspector: boolean;
  showFilmstrip: boolean;
  fontSize: number;
  prefetch: number;
}
export interface Trip {
  id: string;
  name: string;
  description: string | null;
  startAt: number | null;
  endAt: number | null;
  coverAssetId: string | null;
  count: number;
}
export interface StoryItem {
  assetId: Id;
  filename: string;
  mediaKind: MediaKind;
  fileState: FileState;
  date: string | null;
  unknownTimezone: boolean;
  note: string;
}
export interface TripStory {
  trip: Trip;
  items: StoryItem[];
  total: number;
  epoch: number;
  nextOffset: number | null;
}
export interface StoryExportPreview {
  planToken: string;
  expiresAt: number;
  itemCount: number;
  offlineCount: number;
  folder: string;
  includeNotes: boolean;
}
export interface SavedFilter {
  id: string;
  name: string;
  filter: AssetFilter;
  sort: AssetSort;
}
export interface MediaLink {
  id: string;
  primaryAssetId: string;
  secondaryAssetId: string;
  linkType: 'live_photo' | 'raw_jpeg';
  matchMethod: string;
  confidence: number | null;
  confirmed: boolean;
}
export interface TimelineDay {
  date: string | null;
  count: number;
  reviewed: number;
  unknownTimezone: number;
  photos: number;
  videos: number;
}
export interface MapPoint {
  lon: number;
  lat: number;
  count: number;
  assetId: string;
}
export interface ExportReportItem {
  assetId: string;
  destination: string;
  state: string;
  errorCode: string | null;
  sha256: string | null;
}

/** Tauri bridge command registry, arguments always passed as invoke(name, { request }). */
export interface AiStatus {
  enabled: boolean;
  modelAvailable: boolean;
  modelFolderPresent: boolean;
  modelVersion: string;
  profileId: string;
  profileName: string;
  modelBytes: number;
  installedAt: number | null;
  runtime: 'idle' | 'loading' | 'loaded';
  inferenceThreads: number;
  idleReleaseSeconds: number;
  total: number;
  indexedCount: number;
}
export interface AiModelInfo {
  profile: {
    id: string;
    name: string;
    repository: string;
    revision: string;
    license: string;
    embeddingDimension: number;
    imageSize: number;
    maxTextTokens: number;
    files: { path: string; bytes: number; sha256: string; url: string }[];
  };
  card: string;
  licenseText: string;
}
export interface SemanticResults {
  items: { asset: AssetSummary; score: number }[];
  total: number;
  indexedCount: number;
  invalidCount: number;
  modelVersion: string;
  limit: number;
}
export interface AiClusters {
  jobId: Id | null;
  groups: {
    id: Id;
    name: string;
    count: number;
    validCount: number;
    staleCount: number;
    previews: AssetSummary[];
  }[];
  summary: {
    scopeCount: number;
    clusteredCount: number;
    failedCount: number;
    desiredGroups: number;
    actualGroups: number;
  } | null;
  createdAt: number | null;
  filter: AssetFilter;
  modelVersion?: string;
  algorithmVersion?: string;
}
export interface QualityStatus {
  eyesEnabled: boolean;
  detailEnabled: boolean;
  modelAvailable: boolean;
  modelFolderPresent: boolean;
  profileId: string;
  profileName: string;
  modelBytes: number;
  installedAt: number | null;
  runtime: 'idle' | 'loading' | 'loaded';
  inferenceThreads: number;
  idleReleaseSeconds: number;
  total: number;
  eyesCount: number;
  detailCount: number;
  eyeVersion: string;
  detailVersion: string;
}
export interface EyeHints {
  faces: {
    bounds: [number, number, number, number];
    blinkLeft: number;
    blinkRight: number;
    eyeRatioLeft: number;
    eyeRatioRight: number;
    status: 'possibleClosed' | 'possibleBlink' | 'noHint' | 'uncertain';
    reason: 'bothEyes' | 'oneEye' | 'noStrongHint' | 'smallFace' | 'poseOrCrop';
  }[];
  faceLimitReached: boolean;
  width: number;
  height: number;
}
export interface DetailMetrics {
  laplacianVariance: number;
  edgeEnergy: number;
  measuredWidth: number;
  measuredHeight: number;
}
export type QualityObservation = {
  version: string;
  inputKey: string;
  createdAt: number;
  dismissed: boolean;
} & ({ kind: 'eyes'; value: EyeHints } | { kind: 'detail'; value: DetailMetrics });
export interface AssetQuality {
  assetId: Id;
  observations: QualityObservation[];
  staleCount: number;
}
export interface QualityResults {
  items: { asset: AssetSummary; analysis: AssetQuality }[];
  total: number;
  offset: number;
  pageSize: number;
}
export interface Commands {
  get_quality_status: { request: { filter: AssetFilter }; result: QualityStatus };
  set_detail_enabled: { request: { enabled: boolean }; result: { updated: boolean } };
  set_eyes_enabled: { request: { enabled: boolean }; result: { updated: boolean } };
  import_eyes_model: { request: { selectedPath: string; clientMutationId: Id }; result: { jobId: Id } };
  remove_eyes_model: { request: { confirmed: boolean; clientMutationId: Id }; result: { jobId: Id } };
  get_eyes_model_info: { request: void; result: AiModelInfo };
  start_quality_analysis: {
    request: { filter: AssetFilter; useEyes: boolean; useDetail: boolean; clientMutationId: Id };
    result: { jobId: Id };
  };
  get_quality_results: {
    request: { filter: AssetFilter; offset: number; onlyHints: boolean; includeDismissed: boolean };
    result: QualityResults;
  };
  get_asset_quality: { request: { assetId: Id }; result: AssetQuality };
  set_quality_dismissed: {
    request: {
      assetId: Id;
      kind: 'eyes' | 'detail';
      inputKey: string;
      dismissed: boolean;
      clientMutationId: Id;
    };
    result: { updated: boolean };
  };

  get_ai_status: { request: { filter: AssetFilter }; result: AiStatus };
  get_ai_model_info: { request: void; result: AiModelInfo };
  set_ai_enabled: { request: { enabled: boolean }; result: { updated: boolean } };
  import_ai_model: { request: { selectedPath: string; clientMutationId: Id }; result: { jobId: Id } };
  remove_ai_model: { request: { confirmed: boolean; clientMutationId: Id }; result: { jobId: Id } };
  start_ai_index: { request: { filter: AssetFilter; clientMutationId: Id }; result: { jobId: Id } };
  start_ai_cluster: {
    request: { filter: AssetFilter; clientMutationId: Id; groups: number };
    result: { jobId: Id };
  };
  semantic_search: { request: { filter: AssetFilter; text: string; limit: number }; result: SemanticResults };
  get_ai_clusters: { request: { filter: AssetFilter; jobId: string | null }; result: AiClusters };
  get_trip_story: {
    request: { tripId: Id; range: 'all' | 'keep'; offset: number; pageSize: number; epoch: number | null };
    result: TripStory;
  };
  preview_story_export: {
    request: {
      tripId: Id;
      range: 'all' | 'keep';
      title: string;
      description: string;
      includeNotes: boolean;
      destinationPath: string;
    };
    result: StoryExportPreview;
  };
  start_story_export: { request: ExportStartRequest; result: { jobId: Id } };
  query_group_members: {
    request: { groupId: string; cursor: string | null; pageSize: number };
    result: Page<DuplicateGroup['members'][number]>;
  };
  resolve_original: { request: { assetId: string }; result: { resourceUrl: string; mimeType: string } };
  pick_directory: {
    request: { purpose: 'source' | 'export' | 'backup' | 'relink' | 'model' | 'diagnostics' };
    result: { path: string | null };
  };
  pick_backup: { request: void; result: { path: string | null } };
  open_asset: { request: { assetId: string }; result: { opened: boolean } };
  open_asset_location: { request: { assetId: string }; result: { opened: boolean } };
  resolve_playback: { request: { assetId: string }; result: { resourceUrl: string } };
  remove_source: { request: { sourceId: string; confirmed: boolean }; result: { updated: boolean } };
  delete_tag: { request: { tagId: string }; result: { updated: boolean } };
  pause_job: { request: { jobId: string }; result: { accepted: boolean } };
  resume_job: { request: { jobId: string }; result: { accepted: boolean } };
  get_job_errors: { request: { jobId: string }; result: { assetId: string | null; message: string }[] };
  get_export_report: { request: { jobId: string }; result: ExportReport & { items: ExportReportItem[] } };
  prepare_restore: {
    request: { selectedBackupPath: string };
    result: { confirmToken: string; message: string };
  };
  get_settings: { request: void; result: Partial<AppSettings> };
  export_diagnostics: { request: { destinationPath: string }; result: { fileName: string } };
  save_settings: { request: AppSettings; result: { updated: boolean } };
  get_session: { request: void; result: CullSession | null };
  save_session: { request: CullSession; result: { updated: boolean } };
  clear_cache: { request: void; result: { updated: boolean } };
  cache_stats: { request: void; result: { count: number; bytes: number } };
  list_trips: { request: void; result: Trip[] };
  save_trip: {
    request: { id?: string; name: string; description: string; startAt: number | null; endAt: number | null };
    result: { tripId: string };
  };
  delete_trip: { request: { tripId: string }; result: { updated: boolean } };
  add_to_trip: { request: { tripId: string; target: SelectionTarget }; result: { updated: boolean } };
  remove_from_trip: { request: { tripId: string; target: SelectionTarget }; result: { updated: boolean } };
  set_trip_cover: { request: { tripId: string; assetId: string }; result: { updated: boolean } };
  list_saved_filters: { request: void; result: SavedFilter[] };
  save_filter: {
    request: { id?: string; name: string; filter: AssetFilter; sort: AssetSort };
    result: { updated: boolean };
  };
  delete_filter: { request: { filterId: string }; result: { updated: boolean } };
  timeline_stats: { request: { filter: AssetFilter }; result: TimelineDay[] };
  map_assets: { request: { filter: AssetFilter }; result: MapPoint[] };
  map_trails: {
    request: { filter: AssetFilter };
    result: {
      type: 'FeatureCollection';
      features: Array<{
        type: 'Feature';
        properties: { date: string; unknownTimezone: boolean };
        geometry: { type: 'LineString'; coordinates: [number, number][] };
      }>;
      totalPhotos: number;
      includedPhotos: number;
      truncated: boolean;
      unknownTimezone: number;
      skippedInvalidTimes: number;
    };
  };
  get_media_links: { request: { assetId: string }; result: MediaLink[] };
  link_media: {
    request: { assetId: string; secondaryAssetId: string; linkType: 'live_photo' | 'raw_jpeg' };
    result: { updated: boolean };
  };
  unlink_media: { request: { assetId: string; linkId: string }; result: { updated: boolean } };
  get_app_status: {
    request: void;
    result: {
      databaseReady: boolean;
      catalogVersion: number;
      catalogEpoch: number;
      power: { onBattery: boolean | null };
      offlineMode: true;
      watcher: { active: boolean; watchedRoots: number; pollingRoots: number; lastError: string | null };
    };
  };
  get_cull_progress: {
    request: { assetId: string; filter: AssetFilter };
    result: {
      scope: { total: number; reviewed: number };
      day: { label: string; total: number; reviewed: number };
      folder: { label: string; total: number; reviewed: number };
    };
  };
  save_capture_time: {
    request: {
      assetId: string;
      captureLocal: string | null;
      captureTzMin: number | null;
      expectedRevision: number;
      clientMutationId: string;
    };
    result: { updated: boolean; revision: number };
  };
  list_sources: { request: void; result: SourceRoot[] };
  add_source: {
    request: { selectedPath: string; options: SourceOptions };
    result: { sourceId: Id; jobId: Id };
  };
  rescan_source: { request: { sourceId: Id }; result: { jobId: Id } };
  set_source_enabled: { request: { sourceId: Id; enabled: boolean }; result: { updated: boolean } };
  query_assets: { request: QueryAssetsRequest; result: QueryAssetsResult };
  get_asset: { request: { assetId: Id }; result: AssetDetail };
  get_asset_neighbors: {
    request: { assetId: Id; queryFingerprint: string; limit: number };
    result: {
      previousId: Id | null;
      nextId: Id | null;
      items?: AssetSummary[];
      anchorMatches?: boolean;
      total: number;
      position: number | null;
    };
  };
  resolve_preview: {
    request: {
      assetId: Id;
      variant: 'small' | 'medium' | 'screen' | 'poster';
      priority?: 'foreground' | 'prefetch';
    };
    result: PreviewHandle;
  };
  create_selection_snapshot: { request: CreateSelectionSnapshotRequest; result: SelectionSnapshot };
  apply_review_batch: { request: ApplyReviewBatchRequest; result: BatchApplyResult };
  undo_review_batch: { request: { changeBatchId: Id }; result: UndoBatchResult };
  list_tags: { request: void; result: Array<{ id: Id; name: string; color: string | null; count: number }> };
  upsert_tag: { request: { id?: Id; name: string; color: string | null }; result: { tagId: Id } };
  merge_tags: { request: { sourceTagId: Id; targetTagId: Id }; result: { affectedCount: number } };
  apply_tags_batch: {
    request: { target: SelectionTarget; addTagIds: Id[]; removeTagIds: Id[]; clientMutationId: Id };
    result: BatchApplyResult;
  };
  start_duplicate_scan: { request: DuplicateScanRequest; result: { jobId: Id } };
  query_duplicate_groups: {
    request: { kind: 'exact' | 'similar'; cursor: string | null; pageSize: number };
    result: Page<DuplicateGroup>;
  };
  query_burst_groups: {
    request: { filter: AssetFilter; gapSeconds: number; cursor: string | null; pageSize: number };
    result: Page<BurstGroup> & {
      candidateCount: number;
      examinedCount: number;
      skippedInvalidTimes: number;
      truncated: boolean;
    };
  };
  create_burst_snapshot: {
    request: { filter: AssetFilter; gapSeconds: number; groupId: Id };
    result: SelectionSnapshot;
  };
  set_group_representative: { request: { groupId: Id; assetId: Id }; result: { updated: boolean } };
  exclude_group_member: {
    request: { groupId: Id; assetId: Id; excluded: boolean };
    result: { updated: boolean };
  };
  preview_export: { request: ExportPlanRequest; result: ExportPreview };
  list_export_presets: { request: undefined; result: ExportPreset[] };
  save_export_preset: {
    request: { id?: Id; expectedRevision?: number; name: string; options: ExportOptions };
    result: ExportPreset;
  };
  delete_export_preset: { request: { id: Id; expectedRevision: number }; result: { updated: boolean } };
  preview_rename: { request: { target: SelectionTarget; rule: NamingRule }; result: { jobId: Id } };
  get_rename_preview: { request: { jobId: Id; offset: number; pageSize: number }; result: RenamePreview };
  start_export: { request: ExportStartRequest; result: { jobId: Id } };
  list_jobs: { request: { cursor: string | null; pageSize: number }; result: Page<JobSummary> };
  get_job: { request: { jobId: Id }; result: JobSummary };
  cancel_job: { request: { jobId: Id }; result: { accepted: boolean } };
  retry_job: { request: { jobId: Id }; result: { newJobId: Id } };
  backup_catalog: { request: { destinationPath: string }; result: { jobId: Id } };
  restore_catalog: {
    request: { selectedBackupPath: string; confirmToken: string };
    result: { restoredVersion: number };
  };
  preview_relink_source: { request: { sourceId: Id; selectedNewPath: string }; result: RelinkPreview };
  commit_relink_source: {
    request: { sourceId: Id; verificationToken: string };
    result: { updated: boolean };
  };
}
export interface CullSession {
  filter: AssetFilter;
  sort: AssetSort;
  anchorAssetId: string;
  queryFingerprint: string;
  returnPath: string;
  selectedIds?: string[];
  position?: number;
  total?: number;
  scopeFilter?: AssetFilter;
  returnFilter?: AssetFilter;
  returnSort?: AssetSort;
  returnScrollTop?: number;
}
