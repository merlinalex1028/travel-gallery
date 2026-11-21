<script setup lang="ts">
import { ElMessage } from 'element-plus';
import { computed, shallowRef, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api, errorMessage } from '../services/api';
import Icon from './Icon.vue';
const app = useApp(),
  lib = useLibrary(),
  router = useRouter();
// Dismiss the overlay even when WKWebView defers animation frames.
const dialogTransition = { name: 'dialog-fade', css: false };
const path = shallowRef(''),
  recursive = shallowRef(true),
  skipHidden = shallowRef(true),
  exclusions = shallowRef('**/.cache/**, **/node_modules/**'),
  depth = shallowRef(30),
  busy = shallowRef(false);
const overlap = shallowRef<{ sourceCount: number; indexedCount: number; confirmation: string } | null>(null);
const valid = computed(() => path.value && !busy.value);
watch([path, recursive, skipHidden, exclusions, depth, () => app.importOpen], () => {
  overlap.value = null;
});
async function choose() {
  const r = await app.perform(() => api('pick_directory', { purpose: 'source' }));
  if (r?.path) {
    overlap.value = null;
    path.value = r.path;
  }
}
async function start(overlapConfirmation?: string) {
  if (!valid.value) return;
  busy.value = true;
  try {
    const r = await api('add_source', {
      selectedPath: path.value,
      overlapConfirmation,
      options: {
        recursive: recursive.value,
        skipHidden: skipHidden.value,
        excludeGlobs: exclusions.value
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
        followSymlinks: false,
        maxDepth: depth.value,
      },
    });
    path.value = '';
    app.importOpen = false;
    lib.setFilter({ sourceIds: [r.sourceId] });
    await app.refresh();
    await router.push('/library');
    await lib.search();
    ElMessage.success('来源已加入，正在后台建立只读索引');
  } catch (e) {
    const details =
      e && typeof e === 'object' && 'code' in e && e.code === 'SOURCE_OVERLAP' && 'details' in e
        ? e.details
        : null;
    if (
      details &&
      typeof details === 'object' &&
      'sourceCount' in details &&
      typeof details.sourceCount === 'number' &&
      'indexedCount' in details &&
      typeof details.indexedCount === 'number' &&
      'overlapConfirmation' in details &&
      typeof details.overlapConfirmation === 'string'
    ) {
      overlap.value = {
        sourceCount: details.sourceCount,
        indexedCount: details.indexedCount,
        confirmation: details.overlapConfirmation,
      };
    } else {
      overlap.value = null;
      ElMessage.error(errorMessage(e));
    }
  } finally {
    busy.value = false;
  }
}
</script>
<template>
  <ElDialog
    v-model="app.importOpen"
    title="把一段旅程，放进旅图"
    width="540px"
    :close-on-click-modal="false"
    :close-on-press-escape="!busy"
    :show-close="!busy"
    :transition="dialogTransition"
    ><p class="dialog-lead">选择照片所在的目录。旅图只建立索引，照片和视频会留在原处。</p>
    <button class="folder-picker" :disabled="busy" @click="choose">
      <Icon name="folder" :size="28" /><span>{{ path || '选择本地文件夹' }}</span
      ><Icon name="right" />
    </button>
    <div class="form-stack">
      <label class="check-label"
        ><input v-model="recursive" type="checkbox" :disabled="busy" />包含子文件夹</label
      ><label class="check-label"
        ><input v-model="skipHidden" type="checkbox" :disabled="busy" />跳过隐藏文件及文件夹</label
      ><label v-if="recursive"
        >扫描最大深度<input v-model.number="depth" type="number" min="1" max="100" :disabled="busy" /></label
      ><label
        >排除规则 <span class="muted small">用英文逗号分隔</span
        ><input v-model="exclusions" placeholder="**/.cache/**" :disabled="busy"
      /></label>
    </div>
    <div class="notice"><Icon name="shield" />不会移动、修改或删除你的原始文件。扫描可随时暂停或取消。</div>
    <div v-if="overlap" class="form-stack">
      <p class="notice warning" role="alert">
        与 {{ overlap.sourceCount }} 个已有来源重叠。重叠范围内已有 {{ overlap.indexedCount }} 条索引记录，
        继续可能为同一文件建立新的索引记录。可重新选择目录或确认继续。
      </p>
      <div>
        <button class="btn primary" :disabled="!valid" @click="start(overlap.confirmation)">
          确认并开始索引
        </button>
      </div>
    </div>
    <template #footer
      ><button class="btn" :disabled="busy" @click="app.importOpen = false">取消</button
      ><button class="btn primary" :disabled="!valid || !!overlap" @click="start()">
        {{ busy ? '正在加入…' : '开始索引' }}<Icon name="next" :size="16" /></button></template
  ></ElDialog>
</template>
