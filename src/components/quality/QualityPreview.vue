<script setup lang="ts">
import { computed } from 'vue';
import type { AssetQuality } from '../../services/contracts';
import MediaThumbnail from '../MediaThumbnail.vue';
const props = defineProps<{ assetId: string; filename: string; analysis: AssetQuality }>();
const eye = computed(() => props.analysis.observations.find((o) => o.kind === 'eyes'));
</script>
<template>
  <div class="quality-preview">
    <MediaThumbnail :asset-id="assetId" :alt="filename" variant="medium" fit="contain" />
    <svg
      v-if="eye?.kind === 'eyes' && !eye.dismissed"
      :viewBox="`0 0 ${eye.value.width} ${eye.value.height}`"
      preserveAspectRatio="xMidYMid meet"
      aria-hidden="true"
    >
      <g
        v-for="(face, index) in eye.value.faces"
        :key="index"
        :class="{ hint: face.status === 'possibleClosed' || face.status === 'possibleBlink' }"
      >
        <rect
          :x="face.bounds[0] * eye.value.width"
          :y="face.bounds[1] * eye.value.height"
          :width="(face.bounds[2] - face.bounds[0]) * eye.value.width"
          :height="(face.bounds[3] - face.bounds[1]) * eye.value.height"
          vector-effect="non-scaling-stroke"
        />
        <text
          v-if="eye.value.faces.length > 1"
          :x="face.bounds[0] * eye.value.width"
          :y="face.bounds[1] * eye.value.height"
          :font-size="Math.max(18, eye.value.width / 45)"
        >
          {{ index + 1 }}
        </text>
      </g>
    </svg>
  </div>
</template>
<style scoped>
.quality-preview {
  position: relative;
  height: 220px;
  background: var(--surface-raised);
  border-radius: var(--r-sm);
  overflow: hidden;
}
.quality-preview :deep(.thumbnail) {
  height: 100%;
}
.quality-preview svg {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
}
rect {
  fill: none;
  stroke: #eff4fc;
  stroke-width: 1.5;
}
text {
  fill: white;
  paint-order: stroke;
  stroke: #2a3358;
  stroke-width: 2px;
}
.hint rect {
  stroke: #f0dca8;
  stroke-width: 2;
}
</style>
