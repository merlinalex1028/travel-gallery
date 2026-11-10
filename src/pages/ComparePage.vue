<script setup lang="ts">
import { computed, onMounted, reactive, shallowRef } from 'vue';
import { useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { useMediaKeyboard } from '../composables/keyboard';
import { api } from '../services/api';
import type { AssetDetail, ReviewPatch } from '../services/contracts';
import Icon from '../components/Icon.vue';
import EmptyState from '../components/EmptyState.vue';
import MediaStage from '../components/MediaStage.vue';
import ReviewControls from '../components/ReviewControls.vue';
const app = useApp(),
  lib = useLibrary(),
  router = useRouter(),
  assets = shallowRef<AssetDetail[]>([]),
  active = shallowRef(''),
  zoom = shallowRef(1),
  actualPixels = shallowRef(false),
  sync = shallowRef(true),
  pan = shallowRef({ x: 0, y: 0 }),
  busy = shallowRef(false);
onMounted(async () => {
  if (lib.snapshot || lib.selected.size < 2 || lib.selected.size > 4) return;
  await app.perform(async () => {
    assets.value = await Promise.all([...lib.selected].map((assetId) => api('get_asset', { assetId })));
    active.value = assets.value[0]?.id ?? '';
  });
});
type View = { zoom: number; actualPixels: boolean; pan: { x: number; y: number } };
const views = reactive<Record<string, View>>({});
function view(id: string): View {
  return sync.value
    ? { zoom: zoom.value, actualPixels: actualPixels.value, pan: pan.value }
    : (views[id] ??= { zoom: 1, actualPixels: false, pan: { x: 0, y: 0 } });
}
const currentView = computed(() => view(active.value));
function setView(id: string, patch: Partial<View>) {
  if (sync.value) {
    if (patch.zoom !== undefined) zoom.value = patch.zoom;
    if (patch.actualPixels !== undefined) actualPixels.value = patch.actualPixels;
    if (patch.pan) pan.value = patch.pan;
  } else Object.assign(view(id), patch);
}
function togglePixels() {
  setView(active.value, { actualPixels: !currentView.value.actualPixels, zoom: 1, pan: { x: 0, y: 0 } });
}
let actions: Promise<unknown> = Promise.resolve();
function enqueue(action: () => Promise<void>) {
  const task = actions
    .catch(() => {})
    .then(async () => {
      busy.value = true;
      try {
        await app.perform(action);
      } finally {
        busy.value = false;
      }
    });
  actions = task;
  return task;
}
function mark(id: string, value: ReviewPatch | (() => ReviewPatch)) {
  return enqueue(async () => {
    await lib.apply(typeof value === 'function' ? value() : value, { kind: 'explicit', assetIds: [id] });
    const asset = await api('get_asset', { assetId: id });
    assets.value = assets.value.map((item) => (item.id === id ? asset : item));
  });
}
function undo() {
  return enqueue(async () => {
    await lib.undo();
    assets.value = await Promise.all(assets.value.map((asset) => api('get_asset', { assetId: asset.id })));
  });
}
async function winner(id: string) {
  await mark(id, { pickState: 'keep' });
}
useMediaKeyboard((key, e) => {
  if (key === 'undo') {
    e.preventDefault();
    void undo();
    return;
  }
  if (key === 'escape') void router.push('/library');
  if (key === ' ') {
    e.preventDefault();
    togglePixels();
  }
  const targetId = active.value;
  const patch =
    key === 'p'
      ? { pickState: 'keep' as const }
      : key === 'x'
        ? { pickState: 'reject' as const }
        : key === 'u'
          ? { pickState: 'maybe' as const }
          : key === 'f'
            ? () => ({ isFavorite: !assets.value.find((a) => a.id === targetId)?.review.isFavorite })
            : /^[0-5]$/.test(key)
              ? { rating: Number(key) }
              : null;
  if (patch && active.value) {
    e.preventDefault();
    void mark(targetId, patch);
  }
});
</script>
<template>
  <div class="compare-page viewer-page">
    <header class="viewer-top">
      <RouterLink to="/library" class="btn ghost"><Icon name="back" :size="17" />返回图库</RouterLink>
      <div class="viewer-title">
        <strong>多图对比</strong><span>{{ assets.length }} 张照片 · 点击照片切换操作对象</span>
      </div>
      <div class="row">
        <button class="btn ghost" :disabled="!lib.history.length || busy" @click="undo">
          <Icon name="undo" :size="15" />撤销
        </button>
        <label class="check-label"><input v-model="sync" type="checkbox" />同步缩放 / 平移</label
        ><button class="btn ghost" @click="togglePixels">
          {{ currentView.actualPixels ? '1:1 原图' : '适应窗口' }}
        </button>
      </div>
    </header>
    <div
      v-if="assets.length"
      class="compare-grid"
      :style="{ gridTemplateColumns: `repeat(${assets.length <= 2 ? assets.length : 2}, minmax(0,1fr))` }"
    >
      <article
        v-for="asset in assets"
        :key="asset.id"
        class="compare-item"
        :class="{ active: active === asset.id }"
        tabindex="0"
        @click="active = asset.id"
        @focus="active = asset.id"
      >
        <div class="compare-item-heading">
          <strong>{{ asset.filename }}</strong
          ><button class="text-btn" :disabled="busy" @click.stop="winner(asset.id)">
            <Icon name="flag" :size="14" />保留这一张
          </button>
        </div>
        <MediaStage
          :asset="asset"
          :zoom="view(asset.id).zoom"
          :actual-pixels="view(asset.id).actualPixels"
          :sync-pan="view(asset.id).pan"
          @pan="setView(asset.id, { pan: $event })"
          @zoom="setView(asset.id, { zoom: $event })"
        /><ReviewControls :review="asset.review" compact :busy="busy" @patch="mark(asset.id, $event)" />
      </article>
    </div>
    <EmptyState
      v-else
      icon="layers"
      title="先选出想要对比的照片"
      description="在图库中选择 2–4 张照片，然后点击「对比」。"
      action="回到图库"
      @action="router.push('/library')"
    />
    <footer class="viewer-footer">
      <Icon name="shield" :size="13" />只标记你指定的照片，每张照片的状态独立保存。
    </footer>
  </div>
</template>
