import { defineStore } from 'pinia';
import { computed, reactive, shallowRef, watch } from 'vue';
import { api, errorMessage } from '../services/api';
import type {
  AppError,
  AssetDetail,
  AssetFilter,
  AssetSort,
  AssetSummary,
  ApplyReviewBatchRequest,
  CullSession,
  ReviewPatch,
  SelectionSnapshot,
  SelectionTarget,
} from '../services/contracts';
import { ElMessage } from 'element-plus';

export const useLibrary = defineStore('library', () => {
  const filter = reactive<AssetFilter>({}),
    sort = reactive<AssetSort>({ field: 'captureUtcMs', direction: 'desc' });
  const items = shallowRef<AssetSummary[]>([]),
    total = shallowRef(0),
    cursor = shallowRef<string | null>(null),
    fingerprint = shallowRef(''),
    catalogEpoch = shallowRef<number>(),
    loading = shallowRef(false),
    loadingMore = shallowRef(false),
    needsFirstPage = shallowRef(false),
    error = shallowRef('');
  const selected = shallowRef(new Set<string>()),
    snapshot = shallowRef<SelectionSnapshot | null>(null),
    excluded = shallowRef(new Set<string>()),
    snapshotMembers = shallowRef(new Set<string>()),
    anchor = shallowRef(''),
    focusId = shallowRef(''),
    detail = shallowRef<AssetDetail | null>(null),
    scrollTop = shallowRef(0);
  const history = shallowRef<string[]>([]),
    pending = shallowRef(0),
    session = shallowRef<CullSession | null>(null);
  let generation = 0,
    selectionGeneration = 0,
    detailGeneration = 0,
    queue: Promise<unknown> = Promise.resolve();
  const count = computed(() =>
    snapshot.value ? snapshot.value.count - excluded.value.size : selected.value.size,
  );
  const target = computed<SelectionTarget>(() =>
    snapshot.value
      ? { kind: 'snapshot', snapshotId: snapshot.value.id, excludeAssetIds: [...excluded.value] }
      : { kind: 'explicit', assetIds: [...selected.value] },
  );
  const queryUnavailable = computed(() => needsFirstPage.value || loading.value || !!error.value);
  watch(
    [filter, sort],
    () => {
      generation++;
      selectionGeneration++;
      needsFirstPage.value = true;
      loading.value = false;
      loadingMore.value = false;
      cursor.value = null;
      error.value = '';
      scrollTop.value = 0;
    },
    // A caller can restore its saved session anchor after changing the query.
    { deep: true, flush: 'sync' },
  );
  function isSelected(id: string) {
    return snapshot.value ? snapshotMembers.value.has(id) && !excluded.value.has(id) : selected.value.has(id);
  }
  function clearSelection() {
    selectionGeneration++;
    selected.value = new Set();
    snapshot.value = null;
    excluded.value = new Set();
    snapshotMembers.value = new Set();
  }
  function isSnapshotExpired(error: unknown): error is Pick<AppError, 'code' | 'details'> {
    return !!(
      error &&
      typeof error === 'object' &&
      'code' in error &&
      error.code === 'SNAPSHOT_EXPIRED'
    );
  }
  function clearExpiredSnapshot(snapshotId: string | undefined) {
    if (snapshotId && snapshot.value?.id === snapshotId) clearSelection();
  }
  async function search(more = false) {
    if (more && (!cursor.value || loadingMore.value || queryUnavailable.value)) return;
    const seq = more ? generation : ++generation;
    const selectionSnapshotId = snapshot.value?.id;
    const scopeSnapshotId = filter.selectionSnapshotId;
    if (more) loadingMore.value = true;
    else {
      loading.value = true;
      loadingMore.value = false;
    }
    try {
      const result = await api('query_assets', {
        filter: JSON.parse(JSON.stringify(filter)) as AssetFilter,
        sort: { ...sort },
        cursor: more ? cursor.value : null,
        pageSize: 120,
        ...(selectionSnapshotId ? { selectionSnapshotId } : {}),
      });
      if (seq !== generation) return;
      if (selectionSnapshotId && selectionSnapshotId === snapshot.value?.id) {
        if (result.selectionMembership?.snapshotId !== selectionSnapshotId)
          throw new Error('无法核对冻结选区，请刷新结果后重试。');
        snapshotMembers.value = new Set([
          ...(more ? snapshotMembers.value : []),
          ...result.selectionMembership.assetIds,
        ]);
      }
      items.value = more ? [...items.value, ...result.items] : result.items;
      total.value = result.total;
      cursor.value = result.nextCursor;
      fingerprint.value = result.queryFingerprint;
      catalogEpoch.value = result.catalogEpoch;
      needsFirstPage.value = false;
      error.value = '';
    } catch (e) {
      if (seq === generation) {
        error.value = errorMessage(e);
        if (selectionSnapshotId && isSnapshotExpired(e)) {
          const expiredId = e.details?.snapshotId;
          const onlyOneSnapshot = !scopeSnapshotId || scopeSnapshotId === selectionSnapshotId;
          if (expiredId === selectionSnapshotId || (expiredId === undefined && onlyOneSnapshot))
            clearExpiredSnapshot(selectionSnapshotId);
        }
      }
    } finally {
      if (seq === generation) {
        loading.value = false;
        loadingMore.value = false;
      }
    }
  }
  function setFilter(value: AssetFilter) {
    detailGeneration++;
    detail.value = null;
    focusId.value = '';
    anchor.value = '';
    for (const k of Object.keys(filter)) delete filter[k as keyof AssetFilter];
    Object.assign(filter, value);
    scrollTop.value = 0;
    if (!snapshot.value) clearSelection();
  }
  function select(asset: AssetSummary, event?: MouseEvent | KeyboardEvent) {
    if (queryUnavailable.value) return;
    if (snapshot.value && (event?.ctrlKey || event?.metaKey) && !event.shiftKey) {
      if (!snapshotMembers.value.has(asset.id)) {
        ElMessage.info('这张照片不在冻结范围中。请重新选择全部匹配，或单击建立新选区。');
        return;
      }
      selectionGeneration++;
      const next = new Set(excluded.value);
      if (next.has(asset.id)) next.delete(asset.id);
      else next.add(asset.id);
      excluded.value = next;
      focusId.value = asset.id;
      anchor.value = asset.id;
      void inspect(asset.id);
      return;
    }
    focusId.value = asset.id;
    if (snapshot.value) {
      clearSelection();
    }
    const next = new Set(selected.value);
    if (event?.shiftKey && anchor.value) {
      const a = items.value.findIndex((i) => i.id === anchor.value),
        b = items.value.findIndex((i) => i.id === asset.id);
      if (a >= 0)
        for (let n = Math.min(a, b); n <= Math.max(a, b); n++) {
          const item = items.value[n];
          if (item) next.add(item.id);
        }
    } else if (event?.ctrlKey || event?.metaKey) {
      if (next.has(asset.id)) next.delete(asset.id);
      else next.add(asset.id);
    } else {
      next.clear();
      next.add(asset.id);
    }
    selected.value = next;
    anchor.value = asset.id;
    void inspect(asset.id);
  }
  function selectLoaded() {
    if (queryUnavailable.value) return;
    clearSelection();
    selected.value = new Set(items.value.map((i) => i.id));
    ElMessage.info(`已选择当前已加载的 ${selected.value.size} 张，可升级为全部匹配`);
  }
  async function selectAllMatching() {
    if (queryUnavailable.value) return;
    const seq = ++selectionGeneration;
    try {
      const result = await api('create_selection_snapshot', {
        filter: { ...filter },
        sort: { ...sort },
        expectedCatalogEpoch: catalogEpoch.value,
      });
      if (seq !== selectionGeneration) return;
      snapshot.value = result;
      selected.value = new Set();
      excluded.value = new Set();
      snapshotMembers.value = new Set();
      await search();
    } catch (e) {
      if (seq === selectionGeneration) throw e;
    }
  }
  async function inspect(id: string, options: { background?: boolean } = {}) {
    const seq = ++detailGeneration,
      focus = focusId.value;
    try {
      const v = await api('get_asset', { assetId: id });
      if (seq !== detailGeneration || focus !== focusId.value) return false;
      if (detail.value?.id === v.id && detail.value.review.revision > v.review.revision) return false;
      detail.value = v;
      return true;
    } catch (e) {
      if (seq === detailGeneration && focus === focusId.value && !options.background) {
        detail.value = null;
        ElMessage.error(errorMessage(e));
      }
      return false;
    }
  }
  function apply(
    patch: ReviewPatch,
    selection: SelectionTarget = target.value,
    retry?: Pick<ApplyReviewBatchRequest, 'clientMutationId' | 'expectedRevisions'>,
  ) {
    const request: ApplyReviewBatchRequest = {
      target: selection,
      patch: { ...patch },
      clientMutationId: retry?.clientMutationId ?? crypto.randomUUID(),
      ...(retry?.expectedRevisions ? { expectedRevisions: { ...retry.expectedRevisions } } : {}),
    };
    const snapshotId = selection.kind === 'snapshot' ? selection.snapshotId : undefined;
    pending.value++;
    const task = queue
      .catch(() => {})
      .then(async () => {
        try {
          const result = await api('apply_review_batch', request);
          if (!history.value.includes(result.changeBatchId)) {
            history.value = [...history.value.slice(-99), result.changeBatchId];
            if (selection.kind === 'explicit') {
              const ids = new Set(selection.assetIds);
              items.value = items.value.map((i) =>
                ids.has(i.id)
                  ? { ...i, review: { ...i.review, ...patch, revision: i.review.revision + 1 } }
                  : i,
              );
            }
          }
          if (detail.value && selection.kind === 'explicit' && selection.assetIds.includes(detail.value.id))
            await inspect(detail.value.id);
          return result;
        } catch (e) {
          if (isSnapshotExpired(e)) clearExpiredSnapshot(snapshotId);
          throw e;
        }
      });
    queue = task;
    return task.finally(() => {
      pending.value--;
    });
  }
  function applyTags(addTagIds: string[], removeTagIds: string[], selection: SelectionTarget = target.value) {
    const request = {
      target: selection,
      addTagIds: [...addTagIds],
      removeTagIds: [...removeTagIds],
      clientMutationId: crypto.randomUUID(),
    };
    const snapshotId = selection.kind === 'snapshot' ? selection.snapshotId : undefined;
    pending.value++;
    const task = queue
      .catch(() => {})
      .then(async () => {
        try {
          const result = await api('apply_tags_batch', request);
          history.value = [...history.value.slice(-99), result.changeBatchId];
          if (detail.value) await inspect(detail.value.id);
          return result;
        } catch (e) {
          if (isSnapshotExpired(e)) clearExpiredSnapshot(snapshotId);
          throw e;
        }
      });
    queue = task;
    return task.finally(() => {
      pending.value--;
    });
  }
  function undo() {
    pending.value++;
    const task = queue
      .catch(() => {})
      .then(async () => {
        const batch = history.value.at(-1);
        if (!batch) return;
        const res = await api('undo_review_batch', { changeBatchId: batch });
        history.value = history.value.slice(0, -1);
        await search();
        if (focusId.value) await inspect(focusId.value);
        ElMessage.info(
          `已撤销 ${res.restoredCount} 项${res.skippedCount ? `，${res.skippedCount} 项有后续修改，已跳过` : ''}`,
        );
      });
    queue = task;
    return task.finally(() => {
      pending.value--;
    });
  }
  return {
    filter,
    sort,
    items,
    total,
    cursor,
    fingerprint,
    loading,
    loadingMore,
    needsFirstPage,
    error,
    queryUnavailable,
    selected,
    snapshot,
    excluded,
    anchor,
    focusId,
    detail,
    scrollTop,
    history,
    pending,
    session,
    count,
    target,
    isSelected,
    clearSelection,
    search,
    setFilter,
    select,
    selectLoaded,
    selectAllMatching,
    inspect,
    apply,
    applyTags,
    undo,
  };
});
