<script setup lang="ts">
import type { AssetSummary } from '../services/contracts';
import { duration, files, picks } from '../services/format';
import Icon from './Icon.vue';
import MediaThumbnail from './MediaThumbnail.vue';
defineProps<{ asset: AssetSummary; selected: boolean }>();
defineEmits<{
  select: [event: MouseEvent | KeyboardEvent];
  open: [];
  favorite: [];
  context: [event: MouseEvent];
  links: [];
}>();
</script>
<template>
  <article
    class="media-card"
    :class="{ selected, rejected: asset.review.pickState === 'reject' }"
    tabindex="0"
    :aria-label="`${asset.filename}，${picks[asset.review.pickState]}，${asset.review.rating} 星`"
    :aria-selected="selected"
    @click="$emit('select', $event)"
    @dblclick="$emit('open')"
    @keydown.enter.prevent="$emit('open')"
    @keydown.space.prevent="$emit('select', $event)"
    @contextmenu.prevent="$emit('context', $event)"
  >
    <div class="media-image">
      <MediaThumbnail :asset-id="asset.id" :alt="asset.filename" />
      <span class="select-check" :class="{ visible: selected }"
        ><Icon :name="selected ? 'checked' : 'square'" :size="18"
      /></span>
      <span v-if="asset.mediaKind === 'video'" class="media-badge bottom-left"
        ><Icon name="play" :size="12" />{{ duration(asset.durationMs) }}</span
      >
      <button
        v-if="asset.linkedMediaCount"
        class="media-badge top-left"
        @click.stop="$emit('links')"
        :aria-label="`展开 ${asset.linkedMediaCount + 1} 个配套源文件`"
      >
        <Icon name="layers" :size="12" />{{
          asset.linkedMediaType === 'live_photo' ? 'LIVE' : 'RAW + JPEG'
        }}
        · {{ asset.linkedMediaCount + 1 }}
      </button>
      <span
        v-if="asset.mediaKind !== 'video' && !['jpg', 'jpeg', 'png', 'webp'].includes(asset.extension)"
        class="media-badge bottom-left"
        >{{ asset.extension.toUpperCase() }}</span
      >
      <span v-if="asset.fileState !== 'available'" class="media-badge top-right"
        ><Icon name="warning" :size="12" />{{ files[asset.fileState] }}</span
      >
      <button
        class="favorite-btn"
        :class="{ active: asset.review.isFavorite }"
        :aria-label="asset.review.isFavorite ? '取消收藏' : '收藏'"
        @click.stop="$emit('favorite')"
      >
        <Icon name="heart" :size="17" />
      </button>
      <span v-if="asset.review.pickState !== 'unreviewed'" class="pick-badge" :class="asset.review.pickState"
        ><Icon
          :name="
            asset.review.pickState === 'keep'
              ? 'flag'
              : asset.review.pickState === 'reject'
                ? 'close'
                : 'circle'
          "
          :size="12"
        />{{ picks[asset.review.pickState] }}</span
      >
    </div>
    <div class="media-caption">
      <span class="truncate">{{ asset.filename }}</span
      ><span v-if="asset.review.rating" class="stars" :aria-label="`${asset.review.rating} 星`">{{
        '★'.repeat(asset.review.rating)
      }}</span
      ><span v-else class="file-type">{{
        asset.mediaKind === 'video' ? '视频' : asset.extension.toUpperCase()
      }}</span>
    </div>
  </article>
</template>
