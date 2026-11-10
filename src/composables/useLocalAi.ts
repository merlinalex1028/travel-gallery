import { computed, onMounted, onUnmounted, shallowRef, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { ElMessageBox } from 'element-plus';
import { api, errorMessage } from '../services/api';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import type { AiStatus, SemanticResults, AiClusters, AssetFilter } from '../services/contracts';

export function useLocalAi() {
  const app = useApp(),
    lib = useLibrary(),
    route = useRoute(),
    router = useRouter();
  const trip = shallowRef(''),
    range = shallowRef<'all' | 'keep'>('all');
  const filter = computed<AssetFilter>(() => ({
    combineLinked: false,
    ...(trip.value ? { collectionId: trip.value } : {}),
    ...(range.value === 'keep' ? { pickStates: ['keep'] as const } : {}),
  }));
  const currentFilter = () => JSON.parse(JSON.stringify(filter.value)) as AssetFilter;
  const status = shallowRef<AiStatus | null>(null),
    statusError = shallowRef(''),
    actionError = shallowRef('');
  const actionBusy = shallowRef(false),
    starting = shallowRef(false),
    message = shallowRef('');
  const query = shallowRef(''),
    searching = shallowRef(false),
    results = shallowRef<SemanticResults | null>(null),
    searchedText = shallowRef('');
  const groups = shallowRef<AiClusters | null>(null),
    groupCount = shallowRef(3),
    selectedJob = shallowRef<string | null>(typeof route.query.job === 'string' ? route.query.job : null);
  const aiJobs = computed(() => app.jobs.filter((job) => job.operation?.startsWith('ai') || job.operation?.startsWith('eyes') || job.operation === 'qualityAnalyze'));
  const active = computed(() =>
    aiJobs.value.filter((job) => ['queued', 'running', 'paused'].includes(job.status)),
  );
  const modelBusy = computed(() =>
    active.value.some((job) => ['aiImportModel', 'aiRemoveModel'].includes(job.operation ?? '')),
  );
  const completedGroups = computed(() =>
    aiJobs.value.filter((job) => job.operation === 'aiCluster' && job.status === 'completed'),
  );
  const canWork = computed(
    () =>
      !!status.value?.enabled &&
      !!status.value.total &&
      !active.value.length &&
      !starting.value &&
      !actionBusy.value,
  );
  const maximumGroups = computed(() => Math.min(20, status.value?.total ?? 2));
  let alive = true,
    scopeGeneration = 0,
    statusGeneration = 0,
    searchGeneration = 0,
    groupGeneration = 0,
    selectionGeneration = 0;
  let polling: ReturnType<typeof setInterval> | undefined;
  async function refreshStatus() {
    const current = scopeGeneration,
      request = ++statusGeneration;
    try {
      const value = await api('get_ai_status', { filter: currentFilter() });
      if (alive && current === scopeGeneration && request === statusGeneration) {
        status.value = value;
        if (value.total >= 2)
          groupCount.value = Math.min(
            20,
            value.total,
            Math.max(2, Number.isInteger(groupCount.value) ? groupCount.value : 3),
          );
        statusError.value = '';
        if (!value.enabled) {
          searchGeneration++;
          results.value = null;
          searching.value = false;
        }
      }
    } catch (error) {
      if (alive && current === scopeGeneration && request === statusGeneration)
        statusError.value = errorMessage(error);
    }
  }
  async function refreshGroups() {
    const current = scopeGeneration,
      request = ++groupGeneration;
    try {
      const value = await api('get_ai_clusters', { filter: currentFilter(), jobId: selectedJob.value });
      if (alive && current === scopeGeneration && request === groupGeneration) groups.value = value;
    } catch (error) {
      if (alive && current === scopeGeneration && request === groupGeneration)
        actionError.value = errorMessage(error);
    }
  }
  watch([trip, range], () => {
    scopeGeneration++;
    searchGeneration++;
    selectionGeneration++;
    status.value = null;
    results.value = null;
    groups.value = null;
    selectedJob.value = null;
    searching.value = false;
    message.value = '';
    actionError.value = '';
    void refreshStatus();
    void refreshGroups();
  });
  watch(query, () => {
    searchGeneration++;
    searching.value = false;
    results.value = null;
  });
  watch(selectedJob, () => {
    void refreshGroups();
  });
  watch(
    () => aiJobs.value.map((job) => `${job.id}:${job.status}`).join('|'),
    () => {
      void refreshStatus();
      void refreshGroups();
    },
  );
  onMounted(() => {
    void refreshStatus();
    void refreshGroups();
    polling = setInterval(() => {
      void refreshStatus();
    }, 5000);
  });
  onUnmounted(() => {
    alive = false;
    scopeGeneration++;
    searchGeneration++;
    selectionGeneration++;
    if (polling) clearInterval(polling);
  });
  async function toggle() {
    if (!status.value || actionBusy.value) return;
    actionBusy.value = true;
    actionError.value = '';
    try {
      await api('set_ai_enabled', { enabled: !status.value.enabled });
      searchGeneration++;
      results.value = null;
      await refreshStatus();
      await app.refresh();
    } catch (error) {
      if (alive) actionError.value = errorMessage(error);
    } finally {
      if (alive) actionBusy.value = false;
    }
  }
  async function importModel() {
    if (actionBusy.value) return;
    actionBusy.value = true;
    actionError.value = '';
    message.value = '';
    try {
      const selected = await api('pick_directory', { purpose: 'model' });
      if (!alive || !selected.path) return;
      await api('import_ai_model', { selectedPath: selected.path, clientMutationId: crypto.randomUUID() });
      if (alive) {
        message.value = '模型导入已加入后台，完成校验后仍需手动启用。';
        await app.refresh();
        await refreshStatus();
      }
    } catch (error) {
      if (alive) actionError.value = errorMessage(error);
    } finally {
      if (alive) actionBusy.value = false;
    }
  }
  async function removeModel() {
    const confirmed = await ElMessageBox.confirm(
      '移除本机模型及可重建的 AI 索引和内容组？照片源文件、评分、标签、收藏和旅行相册保留。需要时可重新导入原模型包。',
      '移除本地模型',
      { confirmButtonText: '移除模型和 AI 索引', cancelButtonText: '取消', type: 'warning' },
    )
      .then(() => true)
      .catch(() => false);
    if (!confirmed || !alive) return;
    actionBusy.value = true;
    actionError.value = '';
    try {
      await api('remove_ai_model', { confirmed: true, clientMutationId: crypto.randomUUID() });
      if (alive) {
        message.value = '模型和 AI 索引移除任务已加入后台。';
        results.value = null;
        groups.value = null;
        await app.refresh();
        await refreshStatus();
      }
    } catch (error) {
      if (alive) actionError.value = errorMessage(error);
    } finally {
      if (alive) actionBusy.value = false;
    }
  }
  async function startWork(cluster: boolean) {
    if (!canWork.value) return;
    const current = scopeGeneration;
    starting.value = true;
    actionError.value = '';
    try {
      const request = { filter: currentFilter(), clientMutationId: crypto.randomUUID() };
      const result = cluster
        ? await api('start_ai_cluster', { ...request, groups: groupCount.value })
        : await api('start_ai_index', request);
      if (alive && current === scopeGeneration) {
        message.value = cluster
          ? '内容分组已加入后台，将先补齐所选范围的内容索引。'
          : '内容索引已加入后台，可在任务中心暂停、继续或取消。';
        if (cluster) selectedJob.value = result.jobId;
      }
      await app.refresh();
    } catch (error) {
      if (alive && current === scopeGeneration) actionError.value = errorMessage(error);
    } finally {
      if (alive) starting.value = false;
    }
  }
  async function search() {
    if (!status.value?.enabled || searching.value || !query.value.trim()) return;
    const current = ++searchGeneration,
      scope = scopeGeneration,
      text = query.value.trim();
    searching.value = true;
    actionError.value = '';
    results.value = null;
    try {
      const value = await api('semantic_search', { filter: currentFilter(), text, limit: 60 });
      if (alive && current === searchGeneration && scope === scopeGeneration) {
        results.value = value;
        searchedText.value = text;
      }
    } catch (error) {
      if (alive && current === searchGeneration && scope === scopeGeneration)
        actionError.value = errorMessage(error);
    } finally {
      if (alive && current === searchGeneration) searching.value = false;
    }
  }
  async function viewResults() {
    if (!results.value?.items.length) return;
    const current = ++selectionGeneration,
      search = searchGeneration;
    const ids = results.value.items.map((item) => item.asset.id);
    actionError.value = '';
    try {
      const snapshot = await api('create_selection_snapshot', {
        filter: { combineLinked: false },
        sort: { field: 'filename', direction: 'asc' },
        target: { kind: 'explicit', assetIds: ids },
      });
      if (!alive || current !== selectionGeneration || search !== searchGeneration) return;
      if (snapshot.count !== ids.length) throw new Error('部分结果已移除或来源已停用，请重新搜索后再打开。');
      lib.setFilter({ selectionSnapshotId: snapshot.id, combineLinked: false });
      await router.push('/filter?showResults=1');
      await lib.search();
    } catch (error) {
      if (alive && current === selectionGeneration) actionError.value = errorMessage(error);
    }
  }
  async function viewGroup(group: AiClusters['groups'][number]) {
    if (!group.validCount) return;
    lib.setFilter({ aiClusterId: group.id, combineLinked: false });
    await router.push('/filter?showResults=1');
    await lib.search();
  }

  return {
    app,
    trip,
    range,
    status,
    statusError,
    actionError,
    actionBusy,
    starting,
    message,
    query,
    searching,
    results,
    searchedText,
    groups,
    groupCount,
    selectedJob,
    active,
    modelBusy,
    completedGroups,
    canWork,
    maximumGroups,
    refreshStatus,
    refreshGroups,
    toggle,
    importModel,
    removeModel,
    startWork,
    search,
    viewResults,
    viewGroup,
  };
}
