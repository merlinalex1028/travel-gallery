<script setup lang="ts">
import { usePreview } from '../composables/preview';
import Icon from './Icon.vue';
const props = withDefaults(
  defineProps<{
    assetId?: string;
    alt?: string;
    variant?: 'small' | 'medium' | 'screen' | 'poster';
    fit?: 'cover' | 'contain';
  }>(),
  { variant: 'small', fit: 'cover', alt: '' },
);
const { url, reason, loading, retry } = usePreview(() => props.assetId, props.variant);
</script>
<template>
  <div class="thumbnail" :class="{ skeleton: loading }">
    <img
      v-if="url"
      :key="url"
      :src="url"
      :alt="alt"
      :style="{ objectFit: fit }"
      draggable="false"
      @error="retry"
    />
    <div v-else-if="!loading" class="thumb-placeholder" :title="reason">
      <Icon name="imageOff" :size="25" /><span v-if="reason">{{ reason }}</span>
    </div>
  </div>
</template>
