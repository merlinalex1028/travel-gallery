<script setup lang="ts">
import type { AssetReview, ReviewPatch } from '../services/contracts';
import Icon from './Icon.vue';
defineProps<{ review?: AssetReview; busy?: boolean; compact?: boolean }>();
defineEmits<{ patch: [patch: ReviewPatch]; undo: [] }>();
</script>
<template>
  <div class="review-controls" :class="{ compact }">
    <div class="pick-controls">
      <button
        :disabled="busy"
        class="review-btn keep"
        :class="{ active: review?.pickState === 'keep' }"
        :aria-pressed="review?.pickState === 'keep'"
        @click="$emit('patch', { pickState: 'keep' })"
      >
        <Icon name="flag" />保留<kbd>P</kbd></button
      ><button
        :disabled="busy"
        class="review-btn maybe"
        :class="{ active: review?.pickState === 'maybe' }"
        :aria-pressed="review?.pickState === 'maybe'"
        @click="$emit('patch', { pickState: 'maybe' })"
      >
        <Icon name="circle" />待定<kbd>U</kbd></button
      ><button
        :disabled="busy"
        class="review-btn reject"
        :class="{ active: review?.pickState === 'reject' }"
        :aria-pressed="review?.pickState === 'reject'"
        @click="$emit('patch', { pickState: 'reject' })"
      >
        <Icon name="close" />淘汰<kbd>X</kbd>
      </button>
    </div>
    <button
      v-if="review?.pickState && review.pickState !== 'unreviewed'"
      class="text-btn small"
      :disabled="busy"
      @click="$emit('patch', { pickState: 'unreviewed' })"
    >
      清除挑片标记
    </button>
    <div class="rating-controls">
      <button
        v-for="star in 5"
        :key="star"
        class="star-btn"
        :class="{ filled: star <= (review?.rating ?? 0) }"
        :aria-label="`${star} 星`"
        :aria-pressed="review?.rating === star"
        :disabled="busy"
        @click="$emit('patch', { rating: review?.rating === star ? 0 : star })"
      >
        <Icon name="star" :size="19" /></button
      ><button
        class="icon-btn"
        :class="{ liked: review?.isFavorite }"
        :aria-pressed="!!review?.isFavorite"
        :disabled="busy"
        aria-label="切换收藏 F"
        @click="$emit('patch', { isFavorite: !review?.isFavorite })"
      >
        <Icon name="heart" />
      </button>
    </div>
  </div>
</template>
