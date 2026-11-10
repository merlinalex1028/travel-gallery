<script setup lang="ts">
import Icon from './Icon.vue';
withDefaults(
  defineProps<{ icon?: string; title: string; description?: string; action?: string; error?: boolean }>(),
  { icon: 'images' },
);
defineEmits<{ action: [] }>();
</script>
<template>
  <div class="empty-state">
    <div class="empty-icon" :class="{ danger: error }"><Icon :name="icon" :size="34" /></div>
    <h2>{{ title }}</h2>
    <p v-if="description">{{ description }}</p>
    <button v-if="action" class="btn" @click="$emit('action')">{{ action }}</button><slot />
  </div>
</template>
