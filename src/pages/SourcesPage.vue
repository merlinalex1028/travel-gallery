<script setup lang="ts">
import { tr } from '../i18n';

import { useApp } from '../stores/app';
import SourceCard from '../features/SourceCard.vue';
import EmptyState from '../components/EmptyState.vue';
import Icon from '../components/Icon.vue';
import { native as isNative } from '../services/api';
const app = useApp();
</script>
<template>
  <section class="standard-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">YOUR ORIGINALS, ALWAYS YOURS</div>
        <h1>
          {{ tr('来源目录') }}<span class="heading-count">{{ app.sources.length }}</span>
        </h1>
        <p>{{ tr('把分散在硬盘里的回忆，汇集到同一个地方。') }}</p>
      </div>
      <button class="btn primary" @click="app.importOpen = true">
        <Icon name="plus" :size="16" />{{ tr('添加来源目录') }}
      </button>
    </div>
    <div class="notice">
      <Icon name="shield" />{{ tr('来源目录始终只读。断开硬盘后，已有的缩略图和整理记录仍然保留。') }}
    </div>
    <p v-if="isNative && app.watcher" class="muted" role="status">
      <template v-if="app.watcher.lastError">{{ app.watcher.lastError }}</template>
      <template v-else-if="app.watcher.active"
        >{{ tr('自动监听') }} {{ app.watcher.watchedRoots }} {{ tr('个在线目录，变动后增量扫描')
        }}<template v-if="app.watcher.pollingRoots"
          >{{ tr('；其中') }} {{ app.watcher.pollingRoots }} {{ tr('个目录每 30 秒检查') }}</template
        >{{ tr('。') }}</template
      >
      <template v-else>{{ tr('自动监听未启动，可使用目录卡片重新扫描。') }}</template>
    </p>
    <div v-if="app.sources.length" class="source-list">
      <SourceCard v-for="source in app.sources" :key="source.id" :source="source" />
    </div>
    <EmptyState
      v-else
      icon="folder"
      :title="tr('还没有添加来源目录')"
      :description="tr('选择存放照片的本地文件夹或移动硬盘。')"
      :action="tr('添加照片目录')"
      @action="app.importOpen = true"
    /><RouterLink to="/missing" class="text-btn"
      ><Icon name="warning" :size="15" />{{ tr('查看缺失、离线与受限格式') }}</RouterLink
    >
  </section>
</template>
