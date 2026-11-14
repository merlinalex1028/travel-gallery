<script setup lang="ts">
import { computed, shallowRef } from 'vue';
import { useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api } from '../services/api';
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
const valid = computed(() => path.value && !busy.value);
async function choose() {
  const r = await app.perform(() => api('pick_directory', { purpose: 'source' }));
  if (r?.path) path.value = r.path;
}
async function start() {
  busy.value = true;
  await app.perform(async () => {
    const r = await api('add_source', {
      selectedPath: path.value,
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
    app.importOpen = false;
    lib.setFilter({ sourceIds: [r.sourceId] });
    await app.refresh();
    await router.push('/library');
    await lib.search();
  }, '来源已加入，正在后台建立只读索引');
  busy.value = false;
}
</script>
<template>
  <ElDialog
    v-model="app.importOpen"
    title="把一段旅程，放进旅图"
    width="540px"
    :close-on-click-modal="false"
    :transition="dialogTransition"
    ><p class="dialog-lead">选择照片所在的目录。旅图只建立索引，照片和视频会留在原处。</p>
    <button class="folder-picker" @click="choose">
      <Icon name="folder" :size="28" /><span>{{ path || '选择本地文件夹' }}</span
      ><Icon name="right" />
    </button>
    <div class="form-stack">
      <label class="check-label"><input v-model="recursive" type="checkbox" />包含子文件夹</label
      ><label class="check-label"><input v-model="skipHidden" type="checkbox" />跳过隐藏文件及文件夹</label
      ><label v-if="recursive"
        >扫描最大深度<input v-model.number="depth" type="number" min="1" max="100" /></label
      ><label
        >排除规则 <span class="muted small">用英文逗号分隔</span
        ><input v-model="exclusions" placeholder="**/.cache/**"
      /></label>
    </div>
    <div class="notice"><Icon name="shield" />不会移动、修改或删除你的原始文件。扫描可随时暂停或取消。</div>
    <template #footer
      ><button class="btn" @click="app.importOpen = false">取消</button
      ><button class="btn primary" :disabled="!valid" @click="start">
        {{ busy ? '正在加入…' : '开始索引' }}<Icon name="next" :size="16" /></button></template
  ></ElDialog>
</template>
