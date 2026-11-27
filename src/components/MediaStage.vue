<script setup lang="ts">
import { tr } from '../i18n';

import { computed, onBeforeUnmount, onMounted, onUnmounted, shallowRef, useTemplateRef, watch } from 'vue';
import { RouterLink } from 'vue-router';
import { usePreview } from '../composables/preview';
import { api, errorMessage, resourceUrl } from '../services/api';
import { useApp } from '../stores/app';
import type { AssetDetail } from '../services/contracts';
import Icon from './Icon.vue';
import { stageGeometry } from '../services/stage';
const props = withDefaults(
  defineProps<{
    asset: AssetDetail;
    zoom?: number;
    syncPan?: { x: number; y: number };
    actualPixels?: boolean;
  }>(),
  { zoom: 1 },
);
const emit = defineEmits<{ pan: [point: { x: number; y: number }]; zoom: [scale: number] }>();
const app = useApp(),
  screen = usePreview(() => props.asset.id, 'screen'),
  small = usePreview(() => props.asset.id, 'small'),
  pan = shallowRef({ x: 0, y: 0 }),
  drag = shallowRef<{ x: number; y: number; startX: number; startY: number } | null>(null),
  videoUrl = shallowRef<string | null>(null),
  videoReason = shallowRef(''),
  playbackAssetId = shallowRef<string | null>(null),
  resolvingVideo = shallowRef(false);
let playbackAttempt = 0;
let liveHovering = false;
const isLivePhoto = computed(
  () =>
    props.asset.mediaKind === 'image' &&
    props.asset.linkedMediaType === 'live_photo' &&
    (props.asset.linkedMediaCount ?? 0) > 0,
);
const originalUrl = shallowRef<string | null>(null),
  originalReason = shallowRef('');
const url = computed(() => screen.url.value || small.url.value),
  reason = screen.reason,
  loading = computed(() => screen.loading.value || small.loading.value),
  usingSmall = computed(() => !originalUrl.value && !screen.url.value && !!small.url.value);
const needsAuthorization = computed(
  () => app.sources.find((source) => source.id === props.asset.sourceId)?.requiresAuthorization ?? false,
);
const stage = useTemplateRef<HTMLElement>('stage'),
  player = useTemplateRef<HTMLVideoElement>('player'),
  bounds = shallowRef({ width: 1, height: 1 });
const observer = new ResizeObserver(([entry]) => {
  if (entry) bounds.value = { width: entry.contentRect.width, height: entry.contentRect.height };
});
onMounted(() => {
  if (stage.value) observer.observe(stage.value);
});
onBeforeUnmount(stopPlayback);
onUnmounted(() => observer.disconnect());
watch(
  [() => props.asset.id, () => props.actualPixels],
  async ([id, pixels], _, cleanup) => {
    let stale = false;
    cleanup(() => {
      stale = true;
    });
    originalUrl.value = null;
    originalReason.value = '';
    pan.value = { x: 0, y: 0 };
    if (!pixels || props.asset.mediaKind === 'video') return;
    try {
      const r = await api('resolve_original', { assetId: id });
      if (!stale) originalUrl.value = resourceUrl(r.resourceUrl);
    } catch (e) {
      if (!stale) originalReason.value = errorMessage(e);
    }
  },
  { immediate: true, flush: 'sync' },
);
const offset = computed(() => props.syncPan ?? pan.value);
const geometry = computed(() =>
  stageGeometry(
    bounds.value,
    { width: props.asset.width ?? 6000, height: props.asset.height ?? 4000 },
    props.zoom,
    !!originalUrl.value,
    offset.value,
  ),
);
watch(
  [() => props.asset.id, isLivePhoto, () => props.asset.linkedMediaCount],
  () => {
    liveHovering = false;
    stopPlayback();
    pan.value = { x: 0, y: 0 };
  },
  { flush: 'sync' },
);
function down(e: PointerEvent) {
  if (props.asset.mediaKind === 'video') return;
  if (props.zoom <= 1 && !originalUrl.value) return;
  drag.value = { x: e.clientX, y: e.clientY, startX: offset.value.x, startY: offset.value.y };
  (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
}
function move(e: PointerEvent) {
  if (!drag.value) return;
  const p = {
    x: drag.value.startX + (e.clientX - drag.value.x) / geometry.value.width,
    y: drag.value.startY + (e.clientY - drag.value.y) / geometry.value.height,
  };
  pan.value = stageGeometry(
    bounds.value,
    { width: props.asset.width ?? 6000, height: props.asset.height ?? 4000 },
    props.zoom,
    !!originalUrl.value,
    p,
  ).normalized;
  emit('pan', pan.value);
}
async function play() {
  if (resolvingVideo.value) return;
  const id = props.asset.id;
  const attempt = ++playbackAttempt;
  videoReason.value = '';
  videoUrl.value = null;
  playbackAssetId.value = isLivePhoto.value ? null : id;
  resolvingVideo.value = true;
  try {
    if (isLivePhoto.value) {
      const links = await api('get_media_links', { assetId: id });
      if (attempt !== playbackAttempt) return;
      const confirmed = links.filter(
        (link) => link.confirmed && link.linkType === 'live_photo' && link.primaryAssetId === id,
      );
      if (confirmed.length !== 1)
        throw new Error(tr('配对已变化或存在多个动态片段，请在详情的「配套素材」中检查并选择。'));
      playbackAssetId.value = confirmed[0]!.secondaryAssetId;
    }
    const r = await api('resolve_playback', { assetId: playbackAssetId.value! });
    if (attempt === playbackAttempt) videoUrl.value = resourceUrl(r.resourceUrl);
  } catch (e) {
    if (attempt === playbackAttempt) videoReason.value = errorMessage(e);
  } finally {
    if (attempt === playbackAttempt) resolvingVideo.value = false;
  }
}
function releaseVideo() {
  if (!player.value) return;
  player.value.pause();
  player.value.removeAttribute('src');
  player.value.load();
}
function stopPlayback() {
  playbackAttempt++;
  releaseVideo();
  videoUrl.value = null;
  videoReason.value = '';
  playbackAssetId.value = null;
  resolvingVideo.value = false;
}
function enterLivePhoto(event: PointerEvent) {
  if (event.pointerType === 'touch' || liveHovering) return;
  liveHovering = true;
  void play();
}
function leaveLivePhoto() {
  liveHovering = false;
  // Keep an actual decode/source error actionable after moving toward its retry controls.
  if (!videoReason.value) stopPlayback();
}
function activateLivePhoto(event: MouseEvent) {
  // Mouse entry already starts this visit. A click must not replay or interrupt it.
  // Native keyboard clicks (detail 0) and touch taps remain available.
  if (event.detail > 0 && liveHovering) return;
  if (videoUrl.value || resolvingVideo.value) stopPlayback();
  else void play();
}
function videoFailed() {
  releaseVideo();
  videoUrl.value = null;
  videoReason.value = tr('内嵌播放失败，可能是本机不支持此格式，或原文件暂时无法读取。');
}
function imageFailed() {
  if (originalUrl.value) {
    originalUrl.value = null;
    originalReason.value = tr('原图暂时无法读取，已退回屏幕预览。');
  } else if (screen.url.value) screen.retry();
  else small.retry();
}
function reloadPreviews() {
  screen.reload();
  small.reload();
}
</script>
<template>
  <div
    ref="stage"
    class="media-stage"
    :class="{ zoomed: zoom > 1 || !!originalUrl }"
    @pointerdown="down"
    @pointermove="move"
    @pointerup="drag = null"
    @pointercancel="drag = null"
    @wheel.ctrl.prevent="emit('zoom', Math.max(1, Math.min(8, zoom + ($event.deltaY < 0 ? 0.2 : -0.2))))"
  >
    <video
      v-if="videoUrl"
      ref="player"
      :key="asset.id"
      :src="videoUrl"
      :poster="url ?? undefined"
      :controls="!isLivePhoto"
      :muted="isLivePhoto"
      autoplay
      playsinline
      tabindex="0"
      :aria-label="
        isLivePhoto
          ? tr('{0} 的 Live Photo 动态片段', { 0: asset.filename })
          : tr('播放 {0}', { 0: asset.filename })
      "
      class="stage-video"
      @error="videoFailed"
      @ended="isLivePhoto && stopPlayback()"
    /><img
      v-else-if="url"
      :key="asset.id"
      :src="originalUrl || url"
      :alt="asset.filename"
      draggable="false"
      class="stage-image"
      :class="{ 'actual-pixels': !!originalUrl }"
      :style="{
        transform: `translate(${geometry.x}px, ${geometry.y}px) scale(${zoom})`,
        ...(originalUrl ? { width: `${asset.width ?? 6000}px`, height: `${asset.height ?? 4000}px` } : {}),
      }"
      @error="imageFailed"
    />
    <div v-else class="stage-fallback">
      <Icon :name="loading ? 'loader' : 'imageOff'" :size="42" :class="{ spin: loading }" />
      <h3>{{ loading ? tr('正在读取预览…') : tr('此媒体暂时无法预览') }}</h3>
      <p>
        {{
          loading
            ? tr('预览准备好后会自动显示。')
            : reason || asset.preview.reason || tr('原文件可能离线或本机缺少此格式的解码器。')
        }}
      </p>
      <div v-if="!loading" class="stage-fallback-actions" @pointerdown.stop>
        <button class="btn" @click="reloadPreviews">
          <Icon name="refresh" :size="15" />{{ tr('重试预览') }}
        </button>
        <button class="btn" @click="app.perform(() => api('open_asset_location', { assetId: asset.id }))">
          <Icon name="folder" :size="15" />{{ tr('打开所在文件夹') }}
        </button>
        <button class="btn" @click="app.perform(() => api('open_asset', { assetId: asset.id }))">
          <Icon name="external" :size="15" />{{ tr('使用系统应用打开') }}
        </button>
      </div>
    </div>
    <p v-if="usingSmall" class="stage-preview-status" :class="{ 'with-live-photo': isLivePhoto }">
      {{ screen.loading.value ? tr('正在加载清晰预览…') : tr('正在显示缩略预览') }}
    </p>
    <button
      v-if="isLivePhoto"
      type="button"
      class="live-photo-badge"
      :class="{ playing: !!videoUrl }"
      :aria-label="tr('Live Photo：悬停播放动态片段')"
      :aria-pressed="!!videoUrl"
      :aria-busy="resolvingVideo"
      :title="tr('移入播放一次，移开停止；也可按 Enter 或轻点播放')"
      @pointerdown.stop
      @pointerenter="enterLivePhoto"
      @pointerleave="leaveLivePhoto"
      @pointercancel="leaveLivePhoto"
      @blur="leaveLivePhoto"
      @click="activateLivePhoto"
    >
      <svg class="live-photo-symbol" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-dasharray="1 2" />
        <circle cx="12" cy="12" r="7" fill="none" stroke="currentColor" />
        <path d="m10 8 6 4-6 4Z" fill="currentColor" />
      </svg>
      <span>LIVE</span>
    </button>
    <button
      v-if="asset.mediaKind === 'video' && !videoUrl && !videoReason"
      class="video-play"
      :aria-label="resolvingVideo ? tr('正在准备播放') : tr('尝试内嵌播放')"
      :disabled="resolvingVideo"
      @click="play"
    >
      <Icon :name="resolvingVideo ? 'loader' : 'play'" :size="28" :class="{ spin: resolvingVideo }" />
    </button>
    <div class="stage-notices">
      <div v-if="originalReason" class="video-fallback notice" role="status">{{ originalReason }}</div>
      <div
        v-if="needsAuthorization || asset.fileState === 'missing' || asset.fileState === 'offline'"
        class="video-fallback notice"
      >
        {{
          needsAuthorization
            ? tr('来源需要重新授权')
            : asset.fileState === 'offline'
              ? tr('原文件来源离线')
              : tr('原文件缺失')
        }}{{ tr('，已有预览和整理记录保留。') }}
        <RouterLink class="text-btn" to="/sources">{{
          needsAuthorization ? tr('重新授权来源') : tr('重新定位来源')
        }}</RouterLink>
      </div>
      <div v-if="videoReason" class="video-fallback notice" role="status">
        <Icon name="warning" />{{ videoReason
        }}<button class="text-btn" @click="play">{{ tr('重试播放') }}</button
        ><button
          v-if="playbackAssetId"
          class="text-btn"
          @click="app.perform(() => api('open_asset', { assetId: playbackAssetId! }))"
        >
          {{ tr('使用系统应用打开') }}
        </button>
      </div>
    </div>
  </div>
</template>
<style scoped>
.live-photo-badge {
  position: absolute;
  top: 12px;
  left: 12px;
  z-index: 1;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  min-height: 28px;
  padding: 4px 9px 4px 6px;
  border: 1px solid rgb(255 255 255 / 30%);
  border-radius: 999px;
  color: #343941;
  background: rgb(255 255 255 / 82%);
  backdrop-filter: blur(12px);
  font-size: 12px;
  font-weight: 750;
  line-height: 1;
  letter-spacing: 0.04em;
}
.live-photo-badge.playing {
  color: #172b42;
  background: #d8e8f8;
}
.live-photo-symbol {
  width: 20px;
  height: 20px;
}
.stage-preview-status.with-live-photo {
  top: 52px;
}
.stage-notices {
  position: absolute;
  bottom: 10px;
  max-width: 90%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}
.stage-notices > .video-fallback {
  position: static;
  max-width: 100%;
}
.stage-fallback-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px;
}
.stage-preview-status {
  position: absolute;
  top: 12px;
  left: 12px;
  margin: 0;
  padding: 6px 9px;
  border-radius: var(--r-sm);
  background: rgb(14 20 40 / 75%);
  color: #fff;
  font-size: 12px;
  pointer-events: none;
}
</style>
