<script setup lang="ts">
import CustomSelect from '../components/CustomSelect.vue';
import { useViewer } from '../composables/viewer';
import Icon from '../components/Icon.vue';
import MediaStage from '../components/MediaStage.vue';
import MediaThumbnail from '../components/MediaThumbnail.vue';
import ReviewControls from '../components/ReviewControls.vue';
import Inspector from '../components/Inspector.vue';
import EmptyState from '../components/EmptyState.vue';
const v = useViewer();
const {
  app,
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
} = v;
function fullscreen() {
  if (document.fullscreenElement) void document.exitFullscreen();
  else void document.documentElement.requestFullscreen();
}
</script>
<template>
  <div class="viewer-page">
    <header class="viewer-top">
      <button class="btn ghost" @click="v.close"><Icon name="back" :size="17" />返回图库</button>
      <div class="viewer-title">
        <strong>{{ culling ? '挑片工作台' : (asset?.filename ?? '照片查看器') }}</strong
        ><span
          ><template v-if="!culling && !matchesQuery">已不匹配当前筛选（{{ initialTotal }} 张）</template
          ><template v-else>{{ position }} / {{ initialTotal }}</template
          ><span class="dot">·</span>{{ culling ? `本次已处理 ${processed} 张` : '原图只读' }}</span
        >
      </div>
      <div class="viewer-options">
        <CustomSelect
          v-if="culling"
          v-model="scope"
          aria-label="本次挑片范围"
          @change="v.changeScope"
          :options="[
            { value: 'all', label: '当前范围全部' },
            { value: 'unreviewed', label: '仅未处理' },
            { value: 'maybe', label: '仅待定' },
          ]"
        /><button
          class="icon-btn"
          :aria-pressed="app.settings.showFilmstrip"
          :aria-label="app.settings.showFilmstrip ? '隐藏胶片条' : '显示胶片条'"
          @click="
            app.settings.showFilmstrip = !app.settings.showFilmstrip;
            app.perform(app.persist);
          "
        >
          <Icon name="images" /></button
        ><button class="icon-btn" aria-label="显示详情 Tab" @click="showInfo = !showInfo">
          <Icon name="inspector" /></button
        ><button class="icon-btn" aria-label="切换全屏" @click="fullscreen"><Icon name="expand" /></button
        ><button class="icon-btn" aria-label="退出查看器" @click="v.close"><Icon name="close" /></button>
      </div>
    </header>
    <div v-if="culling && progress" class="cull-progress" role="status">
      <span>当前范围 {{ progress.scope.reviewed }} / {{ progress.scope.total }} 已处理</span>
      <span>{{ progress.day.label }} · {{ progress.day.reviewed }} / {{ progress.day.total }}</span>
      <span :title="progress.folder.label"
        >当前文件夹 · {{ progress.folder.reviewed }} / {{ progress.folder.total }}</span
      >
      <progress
        :value="progress.scope.reviewed"
        :max="Math.max(1, progress.scope.total)"
        aria-label="当前挑片范围已处理比例"
      />
    </div>
    <div class="viewer-content">
      <div class="viewer-main">
        <div v-if="error" class="viewer-error" role="alert">
          <Icon name="warning" :size="16" />{{ error
          }}<button v-if="canRetry" class="text-btn" :disabled="busy" @click="v.retryReview">
            {{ retryLabel }}
          </button>
          <button class="text-btn" @click="error = ''">关闭提示</button>
        </div>
        <EmptyState
          v-if="complete"
          icon="success"
          title="这一轮，挑好了"
          description="每一个标记都保存在本机，原始照片始终在原处。"
          action="回到图库"
          @action="v.close"
          ><p v-if="summary">
            保留 {{ summary.keep }} · 待定 {{ summary.maybe }} · 淘汰 {{ summary.reject }} · 未处理
            {{ summary.unreviewed }}
          </p>
          <button v-if="asset" class="btn" @click="complete = false">继续查看最后一张</button></EmptyState
        >
        <div v-else-if="loading && !asset" class="stage-fallback">
          <Icon name="loader" class="spin" :size="32" /><span>正在准备本次挑片范围…</span>
        </div>
        <template v-else-if="asset"
          ><div class="viewer-photo-bar">
            <span>{{ asset.filename }}</span
            ><span
              >{{ saved ? '已保存到本机' : busy ? '正在保存…' : '保存未确认'
              }}<Icon v-if="saved" name="check" :size="13"
            /></span>
          </div>
          <div class="viewer-stage-wrap">
            <button
              class="viewer-arrow prev"
              :disabled="!previousId || busy"
              aria-label="上一张"
              @click="v.navigate(-1)"
            >
              <Icon name="left" :size="25" /></button
            ><MediaStage
              :asset="asset"
              :zoom="zoom"
              :actual-pixels="actualPixels"
              @zoom="zoom = $event"
            /><button
              class="viewer-arrow next"
              :disabled="!nextId || busy"
              aria-label="下一张"
              @click="v.navigate(1)"
            >
              <Icon name="right" :size="25" />
            </button>
            <div v-if="asset.mediaKind !== 'video'" class="zoom-tools">
              <button
                class="icon-btn"
                :disabled="zoom <= 1"
                aria-label="缩小"
                @click="zoom = Math.max(1, zoom - 0.5)"
              >
                <Icon name="zoomOut" :size="17" /></button
              ><button
                class="text-btn"
                @click="
                  actualPixels = !actualPixels;
                  zoom = 1;
                "
              >
                {{
                  actualPixels ? '1:1 原图' : zoom === 1 ? '适应窗口' : `${Math.round(zoom * 100)}%`
                }}</button
              ><button
                class="icon-btn"
                :disabled="zoom >= 8"
                aria-label="放大"
                @click="zoom = Math.min(8, zoom + 0.5)"
              >
                <Icon name="zoomIn" :size="17" />
              </button>
            </div>
          </div>
          <div class="viewer-controls-bar">
            <ReviewControls :review="asset.review" :busy="busy" @patch="v.mark" />
            <div class="viewer-control-extra">
              <label v-if="culling" class="check-label"
                ><input
                  v-model="app.settings.autoAdvance"
                  type="checkbox"
                  @change="app.perform(app.persist)"
                />标记后自动下一张</label
              ><button class="text-btn" :disabled="!lib.history.length || busy" @click="app.perform(v.undo)">
                <Icon name="undo" :size="15" />撤销
              </button>
            </div>
          </div>
          <div v-if="app.settings.showFilmstrip" class="filmstrip">
            <button
              v-for="item in filmstrip"
              :key="item.id"
              :class="{ active: item.id === asset.id }"
              :aria-label="`查看${item.filename}`"
              @click="v.jump(item.id)"
            >
              <MediaThumbnail :asset-id="item.id" :alt="item.filename" /><span
                v-if="item.review.pickState === 'keep'"
                class="film-flag"
                ><Icon name="flag" :size="12"
              /></span>
            </button></div
        ></template>
        <footer class="viewer-footer">
          <Icon name="shield" :size="13" />淘汰只做标记，原始文件不会被删除<span
            >← → 翻图 <span class="dot">·</span> 0–5 评分 <span class="dot">·</span>
            {{ asset?.mediaKind === 'video' ? '聚焦播放器后使用播放快捷键' : 'Space 缩放' }}
            <span class="dot">·</span> Tab 详情</span
          >
        </footer>
      </div>
      <Inspector v-if="showInfo && asset" />
    </div>
  </div>
</template>
