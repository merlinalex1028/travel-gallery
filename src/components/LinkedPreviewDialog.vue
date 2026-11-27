<script setup lang="ts">
import { tr } from '../i18n';

import { ElDialog } from 'element-plus';
import type { AssetDetail } from '../services/contracts';
import { api } from '../services/api';
import { useApp } from '../stores/app';
import MediaStage from './MediaStage.vue';
import Icon from './Icon.vue';
const props = defineProps<{ asset: AssetDetail | null }>();
const emit = defineEmits<{ close: [] }>();
const app = useApp();
function openSystem() {
  const id = props.asset?.id;
  if (id) void app.perform(() => api('open_asset', { assetId: id }));
}
</script>
<template>
  <ElDialog
    :model-value="!!asset"
    :title="tr('配套素材预览')"
    width="min(960px, calc(100vw - 40px))"
    top="6vh"
    append-to-body
    destroy-on-close
    @update:model-value="
      (open) => {
        if (!open) emit('close');
      }
    "
  >
    <template v-if="asset">
      <h3 class="linked-preview-title">{{ asset.filename }}</h3>
      <p v-if="asset.mediaKind === 'video'" class="muted small">
        {{ tr('点击画面中的播放按钮，查看动态片段。') }}
      </p>
      <div class="linked-preview-stage">
        <MediaStage :key="asset.id" :asset="asset" />
      </div>
    </template>
    <template #footer>
      <button v-if="asset" class="btn" @click="openSystem">
        <Icon name="external" :size="15" />{{ tr('使用系统应用打开') }}
      </button>
      <button class="btn primary" @click="emit('close')">{{ tr('关闭预览') }}</button>
    </template>
  </ElDialog>
</template>
