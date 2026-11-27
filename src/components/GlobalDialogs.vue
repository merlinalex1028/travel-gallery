<script setup lang="ts">
import { tr } from '../i18n';

import { computed } from 'vue';
import { useApp } from '../stores/app';
import ImportDialog from './ImportDialog.vue';
import ExportWizard from './ExportWizard.vue';
import RenamePreviewDialog from './RenamePreviewDialog.vue';
const app = useApp();
const shortcuts = computed(() => [
  ['P', tr('保留')],
  ['X', tr('淘汰（仅标记）')],
  ['U', tr('待定')],
  ['0–5', tr('评分')],
  ['F', tr('收藏 / 取消收藏')],
  ['← / →', tr('上一张 / 下一张')],
  ['Space', tr('打开预览 / 适应与 1:1')],
  ['Tab', tr('信息面板')],
  ['Esc', tr('关闭浮层 / 退出查看器')],
  ['⌘ / Ctrl + Z', tr('撤销上次标记')],
  ['⌘ / Ctrl + A', tr('选择当前已加载页')],
]);
</script>
<template>
  <ImportDialog /><ExportWizard /><RenamePreviewDialog /><ElDialog
    v-model="app.helpOpen"
    :title="tr('让挑片更顺手')"
    width="480px"
    ><p class="dialog-lead">{{ tr('在输入框、菜单和弹窗中，照片快捷键会暂停响应。') }}</p>
    <div class="shortcut-list">
      <div v-for="[key, label] in shortcuts" :key="key">
        <span>{{ label }}</span
        ><kbd>{{ key }}</kbd>
      </div>
    </div>
    <div class="notice">{{ tr('P / X / U、评分、收藏彼此独立。淘汰只改变资料库标记。') }}</div></ElDialog
  >
</template>
