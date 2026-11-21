<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, shallowRef, useTemplateRef, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { ElMessageBox } from 'element-plus';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { useMediaKeyboard } from '../composables/keyboard';
import { api } from '../services/api';
import type { AssetFilter, AssetSummary } from '../services/contracts';
import Icon from '../components/Icon.vue';
import MediaGrid from '../components/MediaGrid.vue';
import EmptyState from '../components/EmptyState.vue';
import Inspector from '../components/Inspector.vue';
import LibraryToolbar from '../features/LibraryToolbar.vue';
import FilterPanel from '../features/FilterPanel.vue';
import SelectionBar from '../features/SelectionBar.vue';
const app = useApp(),
  lib = useLibrary(),
  route = useRoute(),
  router = useRouter(),
  filterOpen = shallowRef(route.path === '/filter' && route.query.showResults !== '1');
const context = shallowRef<{ x: number; y: number } | null>(null);
const contextMenu = useTemplateRef<HTMLElement>('contextMenu');
let contextTrigger: HTMLElement | null = null;
const groupBy = shallowRef<'day' | 'month' | 'none'>('day');
watch(
  () => lib.pending,
  (current, previous) => {
    if (!current && previous) void lib.search();
  },
);
const trip = computed(() => app.trips.find((t) => t.id === route.params.id));
const emptySource = computed(() => {
  const { sourceIds } = lib.filter;
  if (route.path !== '/library' || sourceIds?.length !== 1 || Object.keys(lib.filter).length !== 1)
    return undefined;
  return app.sources.find(
    (source) =>
      source.id === sourceIds[0] &&
      source.isEnabled &&
      source.isOnline &&
      source.lastScanAt != null &&
      source.totalAssets === 0,
  );
});
const title = computed(
  () =>
    trip.value?.name ??
    (lib.filter.favorite
      ? '我的收藏'
      : lib.filter.pickStates?.[0] === 'keep'
        ? '已保留'
        : lib.filter.pickStates?.[0] === 'unreviewed'
          ? '未整理'
          : route.path === '/filter'
            ? '筛选结果'
            : '全部媒体'),
);
onMounted(() => {
  if (route.params.id) lib.setFilter({ collectionId: String(route.params.id) });
  void lib.search();
});
watch(
  () => route.params.id,
  (id, previousId) => {
    if (id) lib.setFilter({ collectionId: String(id) });
    else if (previousId && lib.filter.collectionId) lib.setFilter({});
    void lib.search();
  },
);
function setFilter(filter: AssetFilter) {
  lib.setFilter(filter);
  void lib.search();
}
async function open(asset: AssetSummary) {
  if (lib.queryUnavailable) return;
  lib.focusId = asset.id;
  lib.session = {
    filter: { ...lib.filter },
    sort: { ...lib.sort },
    anchorAssetId: asset.id,
    queryFingerprint: lib.fingerprint,
    returnPath: route.fullPath,
    position: lib.items.findIndex((a) => a.id === asset.id) + 1,
    total: lib.total,
  };
  await router.push(`/viewer/${asset.id}`);
}
async function cull() {
  if (!lib.count && (lib.queryUnavailable || !lib.total)) return;
  await app.perform(async () => {
    const returnFilter = { ...lib.filter },
      returnSort = { ...lib.sort },
      returnScrollTop = lib.scrollTop;
    try {
      const hasSelection = !!lib.count;
      const membership = await api('create_selection_snapshot', {
        filter: hasSelection ? {} : { ...lib.filter },
        sort: { ...lib.sort },
        ...(hasSelection ? { target: lib.target } : {}),
      });
      const scopeFilter: AssetFilter = { selectionSnapshotId: membership.id };
      lib.setFilter(scopeFilter);
      await lib.search();
      if (lib.error) throw new Error(lib.error);
      const fallback = lib.items[0];
      if (!fallback) throw new Error('此范围目前没有媒体，请刷新图库。');
      const filter: AssetFilter = { ...scopeFilter, pickStates: ['unreviewed'] };
      lib.setFilter(filter);
      await lib.search();
      if (lib.error) throw new Error(lib.error);
      lib.session = {
        filter,
        scopeFilter,
        sort: { ...lib.sort },
        anchorAssetId: lib.items[0]?.id ?? fallback.id,
        queryFingerprint: lib.fingerprint,
        returnPath: route.fullPath,
        returnFilter,
        returnSort,
        returnScrollTop,
        position: 1,
        total: lib.total,
      };
      await router.push('/cull');
    } catch (error) {
      lib.setFilter(returnFilter);
      Object.assign(lib.sort, returnSort);
      lib.scrollTop = returnScrollTop;
      lib.session = null;
      await lib.search();
      throw error;
    }
  });
}
async function saveFilter() {
  const answer = await ElMessageBox.prompt('保存当前筛选规则，之后自动显示符合条件的照片。', '保存智能筛选', {
    inputPlaceholder: '例如：四星以上的保留照片',
    inputValidator: (value) => !!value?.trim() || '请输入名称',
  }).catch(() => null);
  if (answer)
    await app.perform(async () => {
      await api('save_filter', {
        name: answer.value.trim(),
        filter: { ...lib.filter },
        sort: { ...lib.sort },
      });
      await app.refresh();
    }, '智能筛选已保存');
}
function compare() {
  if (lib.count >= 2 && lib.count <= 4 && !lib.snapshot) void router.push('/compare');
}
async function showContext(asset: AssetSummary, event: MouseEvent) {
  if (lib.queryUnavailable) return;
  contextTrigger =
    event.target instanceof HTMLElement ? event.target.closest<HTMLElement>('[tabindex],button') : null;
  if (!lib.isSelected(asset.id)) lib.select(asset);
  context.value = {
    x: Math.min(event.clientX, innerWidth - 220),
    y: Math.min(event.clientY, innerHeight - 235),
  };
  await nextTick();
  contextMenu.value?.querySelector<HTMLButtonElement>('button')?.focus();
}
function closeContext() {
  if (contextMenu.value?.contains(document.activeElement)) contextTrigger?.focus();
  context.value = null;
}
onMounted(() => {
  document.addEventListener('click', closeContext);
});
onUnmounted(() => {
  document.removeEventListener('click', closeContext);
});
useMediaKeyboard((key, event) => {
  if (key === 'select-all') {
    event.preventDefault();
    lib.selectLoaded();
    return;
  }
  if (key === 'escape') {
    closeContext();
    return;
  }
  if (key === 'undo') {
    event.preventDefault();
    void app.perform(lib.undo);
    return;
  }
  if (key === ' ' && lib.focusId) {
    event.preventDefault();
    const a = lib.items.find((i) => i.id === lib.focusId);
    if (a) void open(a);
  }
  if (!lib.count) return;
  const patch =
    key === 'p'
      ? { pickState: 'keep' as const }
      : key === 'x'
        ? { pickState: 'reject' as const }
        : key === 'u'
          ? { pickState: 'maybe' as const }
          : /^[0-5]$/.test(key)
            ? { rating: Number(key) }
            : key === 'f'
              ? { isFavorite: !lib.detail?.review.isFavorite }
              : null;
  if (patch) {
    event.preventDefault();
    void app.perform(() => lib.apply(patch));
  }
});
</script>
<template>
  <div class="library-page">
    <div class="library-primary">
      <div class="page-heading">
        <div>
          <div class="eyebrow">YOUR JOURNEY, FRAME BY FRAME</div>
          <h1>
            {{ title
            }}<span v-if="!lib.queryUnavailable" class="heading-count">{{ lib.total.toLocaleString() }}</span>
          </h1>
          <p>{{ trip?.description ?? '把沿途的风景，整理成值得珍藏的回忆。' }}</p>
        </div>
        <div class="heading-actions">
          <template v-if="trip"
            ><button
              class="btn"
              :disabled="lib.selected.size !== 1 || !!lib.snapshot"
              @click="
                app.perform(async () => {
                  await api('set_trip_cover', { tripId: trip!.id, assetId: [...lib.selected][0]! });
                  await app.refresh();
                }, '旅行封面已更新')
              "
            >
              设为封面</button
            ><button
              class="btn"
              :disabled="!lib.count"
              @click="
                app.perform(async () => {
                  await api('remove_from_trip', { tripId: trip!.id, target: lib.target });
                  await app.refresh();
                  await lib.search();
                  lib.clearSelection();
                }, '已移出相册，照片保留')
              "
            >
              移出旅行
            </button></template
          >
          <button
            class="btn"
            :disabled="!lib.history.length || !!lib.pending"
            title="撤销上次修改"
            @click="app.perform(lib.undo)"
          >
            <Icon name="undo" :size="15" />撤销</button
          ><button class="btn" :disabled="!lib.count && (lib.queryUnavailable || !lib.total)" @click="cull">
            <Icon name="flag" :size="15" />开始挑片
          </button>
        </div>
      </div>
      <LibraryToolbar @filter="filterOpen = true" @save="saveFilter" /><SelectionBar
        @cull="cull"
        @compare="compare"
      />
      <div class="gallery-heading">
        <span
          ><Icon name="calendar" :size="14" /><select
            v-model="groupBy"
            class="group-select"
            aria-label="日期分组"
          >
            <option value="day">按日分组</option>
            <option value="month">按月分组</option>
            <option value="none">不分组</option>
          </select></span
        >
        <div>
          <span v-if="lib.pending" class="muted">正在保存…</span
          ><span v-else-if="lib.loading" class="muted">正在更新匹配结果…</span
          ><span v-else-if="!lib.error" class="muted">{{ lib.items.length }} 张已加载</span
          ><button class="icon-btn" aria-label="刷新图库" @click="lib.search()">
            <Icon name="refresh" :size="14" />
          </button>
        </div>
      </div>
      <EmptyState
        v-if="lib.error"
        icon="warning"
        title="暂时无法加载媒体"
        :description="lib.error"
        action="刷新结果"
        error
        @action="lib.search()"
      />
      <div v-else-if="lib.loading && !lib.items.length" class="gallery-skeleton">
        <div v-for="i in 9" :key="i" class="skeleton" />
      </div>
      <EmptyState
        v-else-if="!lib.items.length && emptySource"
        icon="folder"
        title="此目录没有可索引的媒体"
        description="可选择另一个照片目录。这个来源仍可在来源管理中重新扫描。"
        action="换目录"
        @action="app.importOpen = true"
      />
      <EmptyState
        v-else-if="!lib.items.length"
        :title="
          app.total ? '这里还没有匹配的照片' : app.running.length ? '正在寻找沿途的风景…' : '从一段旅程开始'
        "
        :description="
          app.total
            ? '试试调整筛选条件，或清空搜索。'
            : '添加照片所在的目录，原图留在原处，旅图帮你有序收藏。'
        "
        :action="app.total ? '清空筛选' : '添加照片目录'"
        @action="app.total ? setFilter({}) : (app.importOpen = true)"
      /><MediaGrid
        v-else
        :inert="lib.queryUnavailable"
        :aria-busy="lib.loading"
        :items="lib.items"
        :density="app.settings.density"
        :has-more="!!lib.cursor"
        :loading-more="lib.loadingMore"
        :is-selected="lib.isSelected"
        :initial-scroll="lib.scrollTop"
        :group-by="lib.sort.field === 'captureUtcMs' ? groupBy : 'none'"
        @select="lib.select"
        @open="open"
        @links="
          (asset) => {
            if (lib.queryUnavailable) return;
            lib.select(asset);
            app.settings.showInspector = true;
          }
        "
        @favorite="
          !lib.queryUnavailable &&
          app.perform(() =>
            lib.apply({ isFavorite: !$event.review.isFavorite }, { kind: 'explicit', assetIds: [$event.id] }),
          )
        "
        @load-more="lib.search(true)"
        @context="showContext"
        @scroll="lib.scrollTop = $event"
      />
    </div>
    <Inspector v-if="app.settings.showInspector" /><FilterPanel
      v-model="filterOpen"
      :filter="lib.filter"
      @apply="setFilter"
    /><Teleport to="body"
      ><div
        v-if="context"
        ref="contextMenu"
        role="menu"
        aria-label="选中媒体操作"
        class="context-menu"
        :style="{ left: `${context.x}px`, top: `${context.y}px` }"
        @click.stop
        @keydown.esc.stop.prevent="closeContext"
      >
        <span>操作 {{ lib.count }} 张选中媒体</span
        ><button
          role="menuitem"
          @click="
            () => {
              app.perform(() => lib.apply({ pickState: 'keep' }));
              closeContext();
            }
          "
        >
          <Icon name="flag" />标记保留</button
        ><button
          role="menuitem"
          @click="
            () => {
              app.perform(() => lib.apply({ pickState: 'reject' }));
              closeContext();
            }
          "
        >
          <Icon name="close" />标记淘汰</button
        ><button
          role="menuitem"
          @click="
            () => {
              app.perform(() => lib.apply({ isFavorite: true }));
              closeContext();
            }
          "
        >
          <Icon name="heart" />加入收藏</button
        ><button
          role="menuitem"
          @click="
            () => {
              cull();
              closeContext();
            }
          "
        >
          <Icon name="play" />开始挑片</button
        ><button
          role="menuitem"
          @click="
            () => {
              app.exportOpen = true;
              closeContext();
            }
          "
        >
          <Icon name="export" />复制导出
        </button>
      </div></Teleport
    >
  </div>
</template>
