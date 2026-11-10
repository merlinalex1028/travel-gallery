import { computed, onMounted, onUnmounted, shallowRef, watch } from 'vue';
import { useRouter } from 'vue-router';
import { ElMessageBox } from 'element-plus';
import { api, errorMessage } from '../services/api';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import type {
  AiStatus,
  AssetFilter,
  QualityStatus,
  QualityResults,
  QualityObservation,
} from '../services/contracts';
export function useQuality() {
  const app = useApp(),
    lib = useLibrary(),
    router = useRouter();
  const source = shallowRef('');
  const trip = shallowRef(''),
    range = shallowRef<'all' | 'keep'>('all');
  const onlyHints = shallowRef(true),
    includeDismissed = shallowRef(false),
    offset = shallowRef(0);
  const status = shallowRef<QualityStatus | null>(null),
    results = shallowRef<QualityResults | null>(null);
  const statusError = shallowRef(''),
    error = shallowRef(''),
    message = shallowRef('');
  const busy = shallowRef(false),
    loading = shallowRef(false),
    selected = shallowRef(new Set<string>());
  const filter = computed<AssetFilter>(() => ({
    combineLinked: false,
    ...(source.value ? { sourceIds: [source.value] } : {}),
    ...(trip.value ? { collectionId: trip.value } : {}),
    ...(range.value === 'keep' ? { pickStates: ['keep'] as const } : {}),
  }));
  const scope = () => JSON.parse(JSON.stringify(filter.value)) as AssetFilter;
  const active = computed(() =>
    app.jobs.filter(
      (j) =>
        ['queued', 'running', 'paused'].includes(j.status) &&
        (j.operation?.startsWith('ai') ||
          j.operation?.startsWith('eyes') ||
          j.operation === 'qualityAnalyze'),
    ),
  );
  const modelBusy = computed(() =>
    active.value.some((j) => ['eyesImportModel', 'eyesRemoveModel'].includes(j.operation ?? '')),
  );
  const modelStatus = computed<AiStatus | null>(() =>
    status.value
      ? {
          ...status.value,
          enabled: status.value.eyesEnabled,
          indexedCount: status.value.eyesCount,
          modelVersion: status.value.eyeVersion,
        }
      : null,
  );
  const canStart = computed(
    () =>
      !!status.value?.total &&
      !!(status.value.eyesEnabled || status.value.detailEnabled) &&
      !busy.value &&
      !active.value.length &&
      status.value.total <= 100000,
  );
  let alive = true,
    generation = 0,
    statusRequest = 0,
    resultRequest = 0,
    navigationRequest = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  async function refreshStatus() {
    const current = generation,
      request = ++statusRequest;
    try {
      const value = await api('get_quality_status', { filter: scope() });
      if (alive && current === generation && request === statusRequest) {
        status.value = value;
        statusError.value = '';
      }
    } catch (e) {
      if (alive && current === generation && request === statusRequest) statusError.value = errorMessage(e);
    }
  }
  async function refreshResults() {
    const current = generation,
      request = ++resultRequest;
    loading.value = true;
    try {
      const value = await api('get_quality_results', {
        filter: scope(),
        offset: offset.value,
        onlyHints: onlyHints.value,
        includeDismissed: includeDismissed.value,
      });
      if (alive && current === generation && request === resultRequest) {
        if (offset.value > 0 && offset.value >= value.total) {
          offset.value = Math.max(0, Math.floor((value.total - 1) / 60) * 60);
          return;
        }
        results.value = value;
        selected.value = new Set(
          [...selected.value].filter((id) => value.items.some((item) => item.asset.id === id)),
        );
      }
    } catch (e) {
      if (alive && current === generation && request === resultRequest) error.value = errorMessage(e);
    } finally {
      if (alive && request === resultRequest) loading.value = false;
    }
  }
  async function refresh() {
    await Promise.all([refreshStatus(), refreshResults()]);
  }
  watch([source, trip, range, onlyHints, includeDismissed], () => {
    generation++;
    navigationRequest++;
    offset.value = 0;
    status.value = null;
    results.value = null;
    selected.value = new Set();
    error.value = '';
    message.value = '';
    void refresh();
  });
  watch(offset, () => {
    generation++;
    navigationRequest++;
    selected.value = new Set();
    results.value = null;
    void refresh();
  });
  watch(
    () =>
      app.jobs
        .filter((j) => j.operation === 'qualityAnalyze' || j.operation?.startsWith('eyes'))
        .map((j) => `${j.id}:${j.status}:${j.progressDone}`)
        .join('|'),
    () => {
      void refresh();
    },
  );
  onMounted(() => {
    void refresh();
    timer = setInterval(() => {
      void refreshStatus();
    }, 5000);
  });
  onUnmounted(() => {
    alive = false;
    generation++;
    navigationRequest++;
    if (timer) clearInterval(timer);
  });
  async function action(run: () => Promise<unknown>, text = '') {
    if (busy.value) return;
    busy.value = true;
    error.value = '';
    message.value = '';
    try {
      await run();
      if (alive) {
        message.value = text;
        await app.refresh();
        await refresh();
      }
    } catch (e) {
      if (alive) error.value = errorMessage(e);
    } finally {
      if (alive) busy.value = false;
    }
  }
  async function toggleEyes() {
    if (status.value) await action(() => api('set_eyes_enabled', { enabled: !status.value!.eyesEnabled }));
  }
  async function toggleDetail() {
    if (status.value)
      await action(() => api('set_detail_enabled', { enabled: !status.value!.detailEnabled }));
  }
  async function importModel() {
    await action(async () => {
      const result = await api('pick_directory', { purpose: 'model' });
      if (!alive || !result.path) return;
      await api('import_eyes_model', { selectedPath: result.path, clientMutationId: crypto.randomUUID() });
    }, '模型导入任务已提交，校验成功后请手动启用。');
  }
  async function removeModel() {
    if (busy.value) return;
    const confirmed = await ElMessageBox.confirm(
      '移除此闭眼模型和可重建的闭眼分析记录？源照片及人工评分、挑片决定均保留。',
      '移除闭眼模型',
      { confirmButtonText: '移除模型和闭眼记录', cancelButtonText: '取消', type: 'warning' },
    )
      .then(() => true)
      .catch(() => false);
    if (confirmed && alive)
      await action(
        () => api('remove_eyes_model', { confirmed: true, clientMutationId: crypto.randomUUID() }),
        '移除任务已提交。',
      );
  }
  async function start() {
    if (!canStart.value || !status.value) return;
    const request = {
      filter: scope(),
      useEyes: status.value.eyesEnabled,
      useDetail: status.value.detailEnabled,
      clientMutationId: crypto.randomUUID(),
    };
    await action(
      () => api('start_quality_analysis', request),
      '照片检查已加入后台，可在任务中心暂停、继续或取消。',
    );
  }
  async function dismiss(assetId: string, observation: QualityObservation) {
    await action(() =>
      api('set_quality_dismissed', {
        assetId,
        kind: observation.kind,
        inputKey: observation.inputKey,
        dismissed: !observation.dismissed,
        clientMutationId: crypto.randomUUID(),
      }),
    );
  }
  function select(id: string) {
    const next = new Set(selected.value);
    if (next.has(id)) next.delete(id);
    else if (next.size < 4) next.add(id);
    selected.value = next;
  }
  async function compare() {
    if (selected.value.size < 2 || selected.value.size > 4) return;
    lib.clearSelection();
    lib.selected = new Set(selected.value);
    await router.push('/compare');
  }
  async function view(id: string) {
    if (loading.value || !results.value) return;
    const request = ++navigationRequest,
      current = generation;
    const ids = results.value.items
      .filter((item) => item.analysis.observations.length)
      .map((item) => item.asset.id);
    if (!ids.includes(id)) return;
    try {
      const snapshot = await api('create_selection_snapshot', {
        filter: { combineLinked: false },
        sort: { field: 'filename', direction: 'asc' },
        target: { kind: 'explicit', assetIds: ids },
      });
      if (!alive || request !== navigationRequest || current !== generation) return;
      if (snapshot.count !== ids.length) throw new Error('部分结果已变化，请刷新后再打开。');
      lib.setFilter({ selectionSnapshotId: snapshot.id, combineLinked: false });
      await router.push(`/viewer/${id}`);
    } catch (e) {
      if (alive && current === generation) error.value = errorMessage(e);
    }
  }
  return {
    app,
    source,
    trip,
    range,
    onlyHints,
    includeDismissed,
    offset,
    status,
    modelStatus,
    results,
    statusError,
    error,
    message,
    busy,
    loading,
    selected,
    active,
    modelBusy,
    canStart,
    refresh,
    toggleEyes,
    toggleDetail,
    importModel,
    removeModel,
    start,
    dismiss,
    select,
    compare,
    view,
  };
}
