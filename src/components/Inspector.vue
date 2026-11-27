<script setup lang="ts">
import { tr } from '../i18n';

import { computed, shallowRef, watch } from 'vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api } from '../services/api';
import { bytes, duration, files } from '../services/format';
import Icon from './Icon.vue';
import MediaThumbnail from './MediaThumbnail.vue';
import type { AssetDetail } from '../services/contracts';
import ReviewControls from './ReviewControls.vue';
import QualityInspector from './quality/QualityInspector.vue';
import LinkedPreviewDialog from './LinkedPreviewDialog.vue';
const app = useApp(),
  lib = useLibrary(),
  note = shallowRef(''),
  editingTime = shallowRef(false),
  timeInput = shallowRef(''),
  offsetInput = shallowRef(''),
  timeRevision = shallowRef(0),
  savingTime = shallowRef(false),
  links = shallowRef<Awaited<ReturnType<typeof loadLinks>>>([]);
const linkedAssets = shallowRef<AssetDetail[]>([]);
const linkedPreview = shallowRef<AssetDetail | null>(null);
async function loadLinks(id: string) {
  return api('get_media_links', { assetId: id });
}
watch(
  () => lib.detail,
  async (a, previous, cleanup) => {
    let stale = false;
    cleanup(() => {
      stale = true;
    });
    if (a?.id !== previous?.id) {
      linkedPreview.value = null;
      note.value = a?.review.note ?? '';
      editingTime.value = false;
    } else if (note.value === (previous?.review.note ?? '')) note.value = a?.review.note ?? '';
    links.value = [];
    linkedAssets.value = [];
    if (a) {
      const result = await app.perform(() => loadLinks(a.id));
      if (stale || !result) return;
      links.value = result;
      const ids = [...new Set(result.flatMap((link) => [link.primaryAssetId, link.secondaryAssetId]))].filter(
        (id) => id !== a.id,
      );
      const files = await app.perform(() => Promise.all(ids.map((assetId) => api('get_asset', { assetId }))));
      if (!stale && files) linkedAssets.value = files;
    }
  },
  { immediate: true },
);
const asset = computed(() => lib.detail);
const visibleMetadata = computed(() =>
  Object.entries(asset.value?.metadata ?? {}).filter(([key]) => !key.startsWith('_')),
);
function editTime() {
  const a = asset.value;
  if (!a) return;
  timeInput.value = a.captureLocal?.replace(' ', 'T') ?? '';
  const offset = a.captureTzMin;
  timeRevision.value = a.captureTimeCorrection?.revision ?? 0;
  offsetInput.value =
    offset === null
      ? ''
      : `${offset < 0 ? '-' : '+'}${String(Math.floor(Math.abs(offset) / 60)).padStart(2, '0')}:${String(Math.abs(offset) % 60).padStart(2, '0')}`;
  editingTime.value = true;
}
async function saveTime(restore = false) {
  const a = asset.value;
  if (!a || savingTime.value) return;
  savingTime.value = true;
  try {
    await app.perform(
      async () => {
        let offset: number | null = null;
        if (!restore && offsetInput.value.trim()) {
          const match = /^([+-])(\d{2}):(\d{2})$/.exec(offsetInput.value.trim());
          if (!match || Number(match[3]) > 59)
            throw new Error(tr('时区请填写 +08:00、-05:30，或留空表示未知'));
          offset = (Number(match[2]) * 60 + Number(match[3])) * (match[1] === '-' ? -1 : 1);
          if (Math.abs(offset) > 840) throw new Error(tr('时区偏移不能超过 14 小时'));
        }
        let local = timeInput.value.replace('T', ' ');
        if (local.length === 16) local += ':00';
        await api('save_capture_time', {
          assetId: a.id,
          captureLocal: restore ? null : local,
          captureTzMin: offset,
          expectedRevision: restore ? (a.captureTimeCorrection?.revision ?? 0) : timeRevision.value,
          clientMutationId: crypto.randomUUID(),
        });
        if (lib.focusId === a.id && asset.value?.id === a.id) {
          editingTime.value = false;
          await lib.inspect(a.id);
        }
        await lib.search();
      },
      restore ? tr('已恢复来源拍摄时间') : tr('拍摄时间已修正，仅保存在资料库'),
    );
  } finally {
    savingTime.value = false;
  }
}
function metadataLabel(key: string) {
  return (
    (
      {
        contentIdentifier: tr('Live Photo 内容标识'),
        rotationDegrees: tr('视频旋转角度'),
        perceptualHashSource: tr('相似哈希来源'),
      } as Record<string, string>
    )[key] ?? key
  );
}
function metadataValue(value: unknown) {
  return (
    (
      {
        raw_embedded_preview: tr('RAW 内嵌 JPEG 预览'),
        platform_converted_preview: tr('系统转换预览'),
        decoded_image: tr('方向标准化后的图片'),
      } as Record<string, string>
    )[String(value)] ??
    value ??
    '—'
  );
}
async function toggleTag(id: string, add: boolean) {
  const a = asset.value;
  if (!a) return;
  await app.perform(async () => {
    await lib.applyTags(add ? [id] : [], add ? [] : [id], { kind: 'explicit', assetIds: [a.id] });
    await app.refresh();
  });
}
async function confirmLink(id: string, second: string, type: 'live_photo' | 'raw_jpeg') {
  await app.perform(async () => {
    await api('link_media', { assetId: id, secondaryAssetId: second, linkType: type });
    if (asset.value) {
      await lib.inspect(asset.value.id);
      await lib.search();
    }
  }, tr('配对已确认，仅改变资料库关联'));
}
</script>
<template>
  <aside class="inspector-panel">
    <div class="inspector-heading">
      <strong>{{ tr('照片详情') }}</strong
      ><button class="icon-btn" :aria-label="tr('关闭详情')" @click="app.settings.showInspector = false">
        <Icon name="close" :size="16" />
      </button>
    </div>
    <template v-if="asset"
      ><MediaThumbnail
        class="inspector-preview"
        :asset-id="asset.id"
        :alt="asset.filename"
        variant="medium"
      />
      <h3 class="inspector-filename">{{ asset.filename }}</h3>
      <div class="muted small">
        {{ asset.width ?? '—' }} × {{ asset.height ?? '—' }} <span class="dot">·</span>
        {{ bytes(asset.byteSize) }}
      </div>
      <ReviewControls
        :review="asset.review"
        compact
        @patch="app.perform(() => lib.apply($event, { kind: 'explicit', assetIds: [asset!.id] }))"
      />
      <QualityInspector v-if="asset.mediaKind === 'image'" :asset-id="asset.id" />
      <div class="inspector-section">
        <h4>{{ tr('拍摄信息') }}</h4>
        <dl class="metadata">
          <template v-if="asset.mediaKind === 'video'">
            <dt>{{ tr('视频时长') }}</dt>
            <dd>{{ asset.durationMs === null ? tr('未知') : duration(asset.durationMs) }}</dd>
            <dt>{{ tr('视频编码') }}</dt>
            <dd>{{ asset.codec ?? tr('未知') }}</dd>
          </template>
          <dt>{{ tr('拍摄时间') }}</dt>
          <dd>{{ asset.captureLocal ?? tr('未知') }}</dd>
          <dt>{{ tr('时间来源') }}</dt>
          <dd>
            {{
              asset.captureTimeSource === 'manual'
                ? tr('手动修正 · {0}', { 0: asset.captureTzMin === null ? tr('时区未知') : tr('已指定时区') })
                : asset.captureTimeSource === 'mtime'
                  ? tr('文件修改时间推断')
                  : asset.captureTzMin === null
                    ? tr('EXIF · 时区未知')
                    : tr('媒体原始时间')
            }}
          </dd>
          <template v-if="asset.captureTimeCorrection?.isCorrected">
            <dt>{{ tr('来源原时间') }}</dt>
            <dd>{{ asset.captureTimeCorrection.original?.captureLocal ?? tr('未知') }}</dd>
            <dt>{{ tr('原时间来源') }}</dt>
            <dd>
              {{ asset.captureTimeCorrection.original?.captureTimeSource ?? tr('未知') }} ·
              {{
                asset.captureTimeCorrection.original?.captureTzMin === null
                  ? tr('时区未知')
                  : tr('UTC 偏移 {0} 分钟', { 0: asset.captureTimeCorrection.original?.captureTzMin })
              }}
            </dd>
          </template>
          <dt>{{ tr('设备') }}</dt>
          <dd>{{ asset.cameraModel ?? tr('未知设备') }}</dd>
          <template v-for="[key, value] in visibleMetadata" :key="key"
            ><dt>{{ metadataLabel(String(key)) }}</dt>
            <dd>{{ metadataValue(value) }}</dd></template
          >
        </dl>
        <button v-if="!editingTime" class="text-btn" @click="editTime">
          <Icon name="pencil" :size="13" />{{ tr('修正拍摄时间') }}
        </button>
        <form v-else class="capture-time-form" @submit.prevent="saveTime()">
          <label
            >{{ tr('修正后的时间')
            }}<input
              v-model="timeInput"
              type="datetime-local"
              step="1"
              required
              min="0001-01-01T00:00"
              max="9999-12-31T23:59:59"
          /></label>
          <label
            >{{ tr('UTC 偏移（未知可留空）')
            }}<input v-model="offsetInput" placeholder="+08:00" :aria-label="tr('拍摄时区 UTC 偏移')"
          /></label>
          <p class="muted small">{{ tr('只更新本机资料库，原文件和来源时间保留。') }}</p>
          <div class="button-row">
            <button class="btn primary" :disabled="savingTime" type="submit">{{ tr('保存修正') }}</button
            ><button class="btn" :disabled="savingTime" type="button" @click="editingTime = false">
              {{ tr('取消') }}
            </button>
          </div>
        </form>
        <button
          v-if="asset.captureTimeCorrection?.isCorrected"
          class="text-btn"
          :disabled="savingTime"
          @click="saveTime(true)"
        >
          {{ tr('恢复来源时间') }}
        </button>
      </div>
      <div class="inspector-section">
        <h4>{{ tr('标签') }}</h4>
        <div class="tag-cloud">
          <button
            v-for="tag in app.tags"
            :key="tag.id"
            class="tag-chip"
            :class="{ active: asset.tagIds.includes(tag.id) }"
            :aria-pressed="asset.tagIds.includes(tag.id)"
            :disabled="!!lib.pending"
            @click="toggleTag(tag.id, !asset!.tagIds.includes(tag.id))"
          >
            <span :style="{ background: tag.color ?? '#9068c0' }" class="tag-dot" />{{ tag.name
            }}<Icon v-if="asset.tagIds.includes(tag.id)" name="check" :size="12" /></button
          ><RouterLink to="/tags" class="tag-chip"
            ><Icon name="plus" :size="12" />{{ tr('管理') }}</RouterLink
          >
        </div>
      </div>
      <div class="inspector-section">
        <h4>{{ tr('备注') }}</h4>
        <textarea
          v-model="note"
          rows="3"
          :placeholder="tr('记下这一帧的故事…')"
          :aria-label="tr('照片备注')"
          @change="app.perform(() => lib.apply({ note }, { kind: 'explicit', assetIds: [asset!.id] }))"
        />
      </div>
      <div v-if="links.length" class="inspector-section">
        <h4>{{ tr('配套素材') }}</h4>
        <div v-for="file in linkedAssets" :key="file.id" class="linked-file">
          <MediaThumbnail :asset-id="file.id" :alt="file.filename" />
          <div>
            <strong>{{ file.filename }}</strong
            ><small class="path-text">{{ file.absolutePath }}</small
            ><button
              class="text-btn"
              :aria-label="
                tr('{0}配套文件 {1}', {
                  0: file.mediaKind === 'video' ? tr('播放') : tr('预览'),
                  1: file.filename,
                })
              "
              @click="linkedPreview = file"
            >
              {{ file.mediaKind === 'video' ? tr('在应用中播放') : tr('在应用中预览') }}</button
            ><button class="text-btn" @click="app.perform(() => api('open_asset', { assetId: file.id }))">
              {{ tr('使用系统应用打开') }}
            </button>
          </div>
        </div>
        <div v-for="link in links" :key="link.id" class="link-row">
          <span
            >{{ link.linkType === 'live_photo' ? 'Live Photo' : 'RAW + JPEG' }} ·
            {{ link.confirmed ? tr('已确认') : tr('候选，需确认') }}</span
          ><button
            v-if="!link.confirmed"
            class="text-btn"
            @click="confirmLink(link.primaryAssetId, link.secondaryAssetId, link.linkType)"
          >
            {{ tr('确认配对') }}</button
          ><button
            class="text-btn"
            @click="
              app.perform(async () => {
                await api('unlink_media', { assetId: asset!.id, linkId: link.id });
                await lib.inspect(asset!.id);
                await lib.search();
              })
            "
          >
            {{ tr('解除') }}
          </button>
        </div>
      </div>
      <div class="inspector-section">
        <h4>{{ tr('文件信息') }}</h4>
        <dl class="metadata">
          <dt>{{ tr('状态') }}</dt>
          <dd>{{ files[asset.fileState] }}</dd>
          <dt>{{ tr('原始路径') }}</dt>
          <dd class="path-text">{{ asset.absolutePath }}</dd>
          <template v-if="asset.gpsLat !== null && asset.gpsLon !== null"
            ><dt>GPS</dt>
            <dd>{{ asset.gpsLat?.toFixed(5) }}, {{ asset.gpsLon?.toFixed(5) }}</dd></template
          >
        </dl>
        <button class="btn full" @click="app.perform(() => api('open_asset', { assetId: asset!.id }))">
          <Icon name="external" :size="15" /> {{ tr('使用系统应用打开') }}
        </button>
      </div></template
    >
    <div v-else class="inspector-empty">
      <Icon name="info" :size="28" />
      <p>{{ tr('选择一张照片') }}<br />{{ tr('查看它的拍摄信息') }}</p>
    </div>
  </aside>
  <LinkedPreviewDialog v-if="linkedPreview" :asset="linkedPreview" @close="linkedPreview = null" />
</template>
