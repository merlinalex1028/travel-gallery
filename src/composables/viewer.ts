import { tr } from '../i18n';
import { computed, onMounted, onUnmounted, shallowRef, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api, errorMessage } from '../services/api';
import { useMediaKeyboard } from './keyboard';
import { prefetchPreview } from './preview';
import type {
  AssetDetail,
  AssetFilter,
  AssetSummary,
  Commands,
  PickState,
  ReviewPatch,
} from '../services/contracts';
type ReviewAttempt = {
  assetId: string;
  patch: ReviewPatch;
  mutationId: string;
  revision: number;
  committed: boolean;
  position: number;
  successor: string | null;
  predecessor: string | null;
};
export function useViewer() {
  const app = useApp(),
    lib = useLibrary(),
    router = useRouter(),
    route = useRoute();
  const culling = computed(() => route.path === '/cull'),
    asset = shallowRef<AssetDetail | null>(null),
    error = shallowRef(''),
    loading = shallowRef(true),
    busy = shallowRef(false),
    complete = shallowRef(false),
    zoom = shallowRef(1),
    actualPixels = shallowRef(false),
    showInfo = shallowRef(false),
    scope = shallowRef('all'),
    saved = shallowRef(true),
    matchesQuery = shallowRef(true),
    position = shallowRef(1),
    initialTotal = shallowRef(0),
    processed = shallowRef(0),
    previousId = shallowRef<string | null>(null),
    nextId = shallowRef<string | null>(null);
  const failedReview = shallowRef<ReviewAttempt | null>(null),
    retryNeedsRefresh = shallowRef(false),
    canRetry = computed(() => !!failedReview.value && failedReview.value.assetId === asset.value?.id),
    retryLabel = computed(() => (retryNeedsRefresh.value ? tr('刷新照片') : tr('重试')));
  const filmstrip = shallowRef<AssetSummary[]>([]),
    progress = shallowRef<Commands['get_cull_progress']['result'] | null>(null),
    summary = shallowRef<Record<PickState, number> | null>(null);
  const processedIds = new Set<string>();
  watch(
    () => lib.detail,
    (detail) => {
      if (detail && detail.id === asset.value?.id) asset.value = detail;
    },
  );
  const undoAnchors: Array<{ id: string; added: boolean; position: number }> = [];
  let generation = 0,
    actions: Promise<unknown> = Promise.resolve(),
    destroyed = false;
  function enqueue(action: () => Promise<unknown>) {
    actions = actions.catch(() => {}).then(action);
    void actions.catch((e) => {
      error.value = errorMessage(e);
    });
    return actions;
  }
  function updateQueryPosition(total: number, rank: number | null) {
    if (culling.value) return;
    initialTotal.value = total;
    matchesQuery.value = rank !== null;
    if (rank !== null) position.value = rank;
    if (lib.session) lib.session = { ...lib.session, total, position: position.value };
  }
  async function neighbors() {
    if (!asset.value || !lib.session) return;
    const current = asset.value.id,
      loadGeneration = generation;
    const list = lib.session.selectedIds;
    if (list) {
      const i = list.indexOf(asset.value.id);
      previousId.value = list[i - 1] ?? null;
      nextId.value = list[i + 1] ?? null;
      updateQueryPosition(list.length, i < 0 ? null : i + 1);
    } else {
      const n = await api('get_asset_neighbors', {
        assetId: asset.value.id,
        queryFingerprint: lib.session.queryFingerprint,
        limit: 5,
      });
      if (loadGeneration !== generation || current !== asset.value?.id || destroyed) return;
      previousId.value = n.previousId;
      nextId.value = n.nextId;
      filmstrip.value = n.items ?? [];
      updateQueryPosition(n.total, n.position);
    }
    if (app.settings.prefetch) {
      for (const id of [nextId.value, previousId.value]
        .filter((id): id is string => !!id)
        .slice(0, app.settings.prefetch))
        prefetchPreview(id, 'screen');
    }
  }
  async function load(id: string) {
    const seq = ++generation;
    loading.value = true;
    error.value = '';
    zoom.value = 1;
    actualPixels.value = false;
    try {
      const a = await api('get_asset', { assetId: id });
      if (seq !== generation || destroyed) return;
      failedReview.value = null;
      retryNeedsRefresh.value = false;
      saved.value = true;
      asset.value = a;
      if (!culling.value && route.params.assetId !== id) await router.replace(`/viewer/${id}`);
      lib.focusId = id;
      lib.detail = a;
      complete.value = false;
      await neighbors();
      await refreshProgress();
      if (lib.session) {
        lib.session = { ...lib.session, anchorAssetId: id, position: position.value };
        if (culling.value) await api('save_session', lib.session);
      }
    } catch (e) {
      if (seq === generation) error.value = errorMessage(e);
    } finally {
      if (seq === generation) loading.value = false;
    }
  }
  async function refreshProgress() {
    if (!culling.value || !asset.value || !lib.session) return;
    const id = asset.value.id;
    try {
      const value = await api('get_cull_progress', {
        assetId: id,
        filter: lib.session.scopeFilter ?? lib.session.filter,
      });
      if (!destroyed && asset.value?.id === id) progress.value = value;
    } catch (e) {
      if (!destroyed && asset.value?.id === id) error.value = errorMessage(e);
    }
  }
  async function enter(filter?: AssetFilter) {
    loading.value = true;
    if (filter) {
      lib.setFilter(filter);
      await lib.search();
      if (lib.error) throw new Error(lib.error);
      lib.session = null;
    }
    if (!lib.session) {
      if (culling.value && !filter) lib.session = await api('get_session', undefined);
      if (!lib.session) {
        const returnFilter = { ...lib.filter },
          returnSort = { ...lib.sort },
          returnScrollTop = lib.scrollTop;
        if (culling.value) {
          const membership = await api('create_selection_snapshot', {
            filter: { ...lib.filter },
            sort: { ...lib.sort },
          });
          lib.setFilter({ selectionSnapshotId: membership.id, pickStates: ['unreviewed'] });
        }
        await lib.search();
        if (lib.error) throw new Error(lib.error);
        const first = route.params.assetId ? String(route.params.assetId) : lib.items[0]?.id;
        lib.session = {
          filter: { ...lib.filter },
          sort: { ...lib.sort },
          anchorAssetId: first ?? '',
          queryFingerprint: lib.fingerprint,
          returnPath: '/library',
          returnFilter,
          returnSort,
          returnScrollTop,
          ...(culling.value ? { scopeFilter: { selectionSnapshotId: lib.filter.selectionSnapshotId } } : {}),
          position: 1,
          total: lib.total,
        };
      } else {
        lib.setFilter(lib.session.filter);
        Object.assign(lib.sort, lib.session.sort);
        await lib.search();
        if (lib.error) throw new Error(lib.error);
        lib.session = { ...lib.session, queryFingerprint: lib.fingerprint };
      }
    }
    position.value = lib.session.position ?? 1;
    initialTotal.value = lib.session.total ?? lib.total;
    scope.value =
      lib.session.filter.pickStates?.length === 1 &&
      ['unreviewed', 'maybe'].includes(lib.session.filter.pickStates[0]!)
        ? lib.session.filter.pickStates[0]!
        : 'all';
    if (!lib.total) {
      complete.value = true;
      loading.value = false;
      await loadSummary();
      return;
    }
    let id = route.params.assetId && !filter ? String(route.params.assetId) : lib.session.anchorAssetId;
    if (culling.value) {
      const n = await api('get_asset_neighbors', {
        assetId: id,
        queryFingerprint: lib.session.queryFingerprint,
        limit: 1,
      });
      if (n.anchorMatches === false) id = n.nextId ?? n.previousId ?? lib.items[0]!.id;
    }
    await load(id);
  }
  async function loadSummary() {
    if (!culling.value || !lib.session) return;
    const filter = { ...(lib.session.scopeFilter ?? lib.session.filter), pickStates: undefined };
    const rows = await Promise.all(
      (['keep', 'maybe', 'reject', 'unreviewed'] as PickState[]).map(async (pick) => {
        const result = await api('query_assets', {
          filter: { ...filter, pickStates: [pick] },
          sort: lib.session!.sort,
          cursor: null,
          pageSize: 1,
        });
        return [pick, result.total] as const;
      }),
    );
    summary.value = Object.fromEntries(rows) as Record<PickState, number>;
  }
  async function navigate(direction: number) {
    const id = direction > 0 ? nextId.value : previousId.value;
    if (id) {
      position.value = Math.max(1, position.value + direction);
      await load(id);
    }
  }
  function jump(id: string) {
    return enqueue(async () => {
      const current = filmstrip.value.findIndex((item) => item.id === asset.value?.id);
      const target = filmstrip.value.findIndex((item) => item.id === id);
      if (current < 0 || target < 0 || current === target) return;
      position.value = Math.max(1, position.value + target - current);
      await load(id);
    });
  }
  function mark(value: ReviewPatch | (() => ReviewPatch), retry?: ReviewAttempt) {
    return enqueue(async () => {
      if (!asset.value || complete.value) return;
      const attempt: ReviewAttempt = retry ?? {
        assetId: asset.value.id,
        patch: { ...(typeof value === 'function' ? value() : value) },
        mutationId: crypto.randomUUID(),
        revision: asset.value.review.revision,
        committed: false,
        position: position.value,
        successor: nextId.value,
        predecessor: previousId.value,
      };
      if (attempt.assetId !== asset.value.id) return;
      failedReview.value = null;
      retryNeedsRefresh.value = false;
      error.value = '';
      busy.value = true;
      saved.value = attempt.committed;
      const { assetId: id, patch, successor, predecessor } = attempt;
      try {
        if (!attempt.committed) {
          await lib.apply(
            patch,
            { kind: 'explicit', assetIds: [id] },
            {
              clientMutationId: attempt.mutationId,
              expectedRevisions: { [id]: attempt.revision },
            },
          );
          attempt.committed = true;
          const added = !!patch.pickState && !processedIds.has(id);
          if (added) processedIds.add(id);
          undoAnchors.push({ id, added, position: attempt.position });
          processed.value = processedIds.size;
        }
        saved.value = true;
        asset.value = await api('get_asset', { assetId: id });
        lib.detail = asset.value;
        if (!culling.value) await neighbors();
        if (patch.pickState) await refreshProgress();
        const leaves =
          (scope.value === 'unreviewed' && patch.pickState && patch.pickState !== 'unreviewed') ||
          (scope.value === 'maybe' && patch.pickState && patch.pickState !== 'maybe');
        if (culling.value && (app.settings.autoAdvance || leaves)) {
          if (successor) {
            position.value++;
            await load(successor);
          } else if (leaves && predecessor) await load(predecessor);
          else if (leaves || !successor) {
            complete.value = true;
            await loadSummary();
          }
        }
      } catch (e) {
        failedReview.value = attempt;
        retryNeedsRefresh.value = !!e && typeof e === 'object' && 'code' in e && e.code === 'DB_CONFLICT';
        error.value = errorMessage(e);
      } finally {
        busy.value = false;
      }
    });
  }
  function retryReview() {
    const attempt = failedReview.value;
    if (!attempt || busy.value) return Promise.resolve();
    // Consume the retry before enqueueing so two clicks cannot advance twice.
    failedReview.value = null;
    if (retryNeedsRefresh.value)
      return enqueue(async () => {
        await load(attempt.assetId);
        if (error.value && asset.value?.id === attempt.assetId) {
          failedReview.value = attempt;
          retryNeedsRefresh.value = true;
        }
      });
    return mark(attempt.patch, attempt);
  }
  async function undo() {
    await lib.undo();
    const anchor = undoAnchors.pop();
    if (anchor?.added) processedIds.delete(anchor.id);
    if (anchor) {
      position.value = anchor.position;
      await load(anchor.id);
    } else if (asset.value) await load(asset.value.id);
    complete.value = false;
    processed.value = processedIds.size;
  }
  async function changeScope(value: string) {
    failedReview.value = null;
    retryNeedsRefresh.value = false;
    scope.value = value;
    const session = lib.session;
    const filter = { ...(session?.scopeFilter ?? session?.filter ?? {}) };
    filter.pickStates = value === 'all' ? undefined : [value as 'unreviewed' | 'maybe'];
    lib.setFilter(filter);
    await lib.search();
    if (lib.error) throw new Error(lib.error);
    if (session)
      lib.session = {
        ...session,
        filter,
        queryFingerprint: lib.fingerprint,
        anchorAssetId: lib.items[0]?.id ?? session.anchorAssetId,
        total: lib.total,
        position: 1,
      };
    await enter();
  }
  function close() {
    void actions
      .catch(() => {})
      .then(() => {
        const session = lib.session;
        if (session?.returnFilter) lib.setFilter(session.returnFilter);
        if (session?.returnSort) Object.assign(lib.sort, session.returnSort);
        if (session?.returnScrollTop !== undefined) lib.scrollTop = session.returnScrollTop;
        lib.session = null;
        void router.push(session?.returnPath ?? '/library');
      });
  }
  useMediaKeyboard((key, event) => {
    if (key === 'escape') {
      event.preventDefault();
      close();
    } else if (key === 'arrowright' || key === 'arrowleft') {
      event.preventDefault();
      enqueue(() => navigate(key === 'arrowright' ? 1 : -1));
    } else if (key === ' ') {
      if (asset.value?.mediaKind === 'video') return;
      event.preventDefault();
      actualPixels.value = !actualPixels.value;
      zoom.value = 1;
    } else if (key === 'tab') {
      event.preventDefault();
      showInfo.value = !showInfo.value;
    } else if (key === 'undo') {
      event.preventDefault();
      enqueue(undo);
    } else {
      const patch =
        key === 'p'
          ? { pickState: 'keep' as const }
          : key === 'x'
            ? { pickState: 'reject' as const }
            : key === 'u'
              ? { pickState: 'maybe' as const }
              : key === 'f'
                ? () => ({ isFavorite: !asset.value?.review.isFavorite })
                : /^[0-5]$/.test(key)
                  ? { rating: Number(key) }
                  : null;
      if (patch) {
        event.preventDefault();
        mark(patch);
      }
    }
  });
  onMounted(() => {
    void enter().catch((e) => {
      loading.value = false;
      error.value = errorMessage(e);
    });
  });
  onUnmounted(() => {
    destroyed = true;
    generation++;
  });
  return {
    app,
    jump,
    lib,
    culling,
    asset,
    error,
    loading,
    busy,
    complete,
    zoom,
    actualPixels,
    showInfo,
    scope,
    saved,
    canRetry,
    retryLabel,
    matchesQuery,
    position,
    initialTotal,
    processed,
    previousId,
    nextId,
    filmstrip,
    summary,
    progress,
    load: (id: string) => enqueue(() => load(id)),
    mark,
    retryReview,
    navigate: (direction: number) => enqueue(() => navigate(direction)),
    changeScope: (value: string) => enqueue(() => changeScope(value)),
    close,
    undo: () => enqueue(undo),
  };
}
