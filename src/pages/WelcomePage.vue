<script setup lang="ts">
import { tr } from '../i18n';

import { useApp } from '../stores/app';
import Icon from '../components/Icon.vue';
const app = useApp();
</script>
<template>
  <div class="welcome-page">
    <div class="welcome-art">
      <img class="welcome-landscape" src="/demo/landscape-0.svg" :alt="tr('山野与湖泊的旅行插画')" />
      <div class="welcome-art-caption">
        <span>EVERY FRAME, A MEMORY.</span
        ><strong>{{ tr('让风景留在照片里，') }}<br />{{ tr('让回忆住进旅图。') }}</strong>
      </div>
      <div class="welcome-photo">
        <img src="/demo/landscape-4.svg" :alt="tr('日落中的山峦插画')" /><span>{{
          tr('那些在路上的日子')
        }}</span>
      </div>
    </div>
    <div class="welcome-copy">
      <div class="welcome-brand">
        <img src="/logo.svg" alt="" width="48" height="48" />
        <div class="eyebrow">WELCOME TO TRAVEL GALLERY</div>
      </div>
      <h1>{{ tr('旅程结束，') }}<br />{{ tr('回忆才刚刚开始。') }}</h1>
      <p>
        {{ tr('照片、视频，还有每一次心动。') }}<br />{{ tr('用一个安静的本地空间，收藏你的所有旅途。') }}
      </p>
      <button class="btn primary large-btn" @click="app.importOpen = true">
        <Icon name="plus" />{{ tr('添加照片目录') }}<Icon name="next" :size="17" />
      </button>
      <div class="welcome-promises">
        <span><Icon name="folder" :size="17" />{{ tr('引用原图，不搬动文件') }}</span
        ><span><Icon name="shield" :size="17" />{{ tr('纯本地，无需登录') }}</span
        ><span><Icon name="flag" :size="17" />{{ tr('用 P / X / U，轻松挑片') }}</span>
      </div>
      <div class="welcome-formats">
        JPG · PNG · WebP · TIFF · HEIC · RAW · MOV · MP4<small>{{
          tr('HEIC、RAW 与视频按本机解码能力提供预览，受限格式保留索引。')
        }}</small>
      </div>
    </div>
  </div>
</template>
