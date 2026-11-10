<script setup lang="ts">
import { computed, onMounted, onUnmounted, shallowRef, useTemplateRef, watch } from 'vue';
import { useVirtualizer } from '@tanstack/vue-virtual';
import type { AssetSummary } from '../services/contracts';
import MediaCard from './MediaCard.vue';
const props = defineProps<{
  items: AssetSummary[];
  density: number;
  hasMore: boolean;
  loadingMore: boolean;
  isSelected: (id: string) => boolean;
  initialScroll?: number;
  groupBy?: 'day' | 'month' | 'none';
}>();
const emit = defineEmits<{
  select: [asset: AssetSummary, event: MouseEvent | KeyboardEvent];
  open: [asset: AssetSummary];
  links: [asset: AssetSummary];
  favorite: [asset: AssetSummary];
  context: [asset: AssetSummary, event: MouseEvent];
  loadMore: [];
  scroll: [top: number];
}>();
const viewport = useTemplateRef<HTMLElement>('viewport'),
  width = shallowRef(900);
const columns = computed(() => Math.max(2, Math.floor((width.value - 4) / (props.density + 14))));
const rowHeight = computed(
  () => Math.ceil((width.value - (columns.value - 1) * 16) / columns.value / 1.48) + 45,
);
type GridRow = { key: string; label: string; items: AssetSummary[] };
const gridRows = computed(() => {
  const result: GridRow[] = [];
  let group = '',
    row: GridRow | undefined;
  for (const asset of props.items) {
    const key =
      props.groupBy === 'none'
        ? ''
        : (asset.captureLocal?.slice(0, props.groupBy === 'month' ? 7 : 10) ?? '未知日期');
    if (key !== group || !row) {
      group = key;
      if (key) result.push({ key: `date-${asset.id}`, label: key, items: [] });
      row = undefined;
    }
    if (!row || row.items.length === columns.value) {
      row = { key: asset.id, label: '', items: [] };
      result.push(row);
    }
    row.items.push(asset);
  }
  return result;
});
const virtualizer = useVirtualizer(
  computed(() => ({
    count: gridRows.value.length,
    getScrollElement: () => viewport.value,
    initialOffset: () => props.initialScroll ?? 0,
    estimateSize: (index: number) => (gridRows.value[index]?.label ? 38 : rowHeight.value),
    getItemKey: (index: number) => gridRows.value[index]?.key ?? index,
    overscan: 3,
  })),
);
const rows = computed(() => virtualizer.value.getVirtualItems());
watch(
  () => rows.value.at(-1)?.index,
  (n) => {
    if (n !== undefined && n + 5 >= gridRows.value.length && props.hasMore && !props.loadingMore)
      emit('loadMore');
  },
);
watch([rowHeight, () => props.groupBy], () => virtualizer.value.measure());
watch(
  () => props.initialScroll,
  (top) => {
    if (viewport.value && top !== undefined) viewport.value.scrollTop = top;
  },
  { flush: 'post' },
);
let observer: ResizeObserver | undefined;
onMounted(() => {
  if (!viewport.value) return;
  observer = new ResizeObserver((entries) => {
    const e = entries[0];
    if (e) width.value = e.contentRect.width;
  });
  observer.observe(viewport.value);
  viewport.value.scrollTop = props.initialScroll ?? 0;
});
onUnmounted(() => observer?.disconnect());
</script>
<template>
  <div
    ref="viewport"
    class="media-viewport"
    @scroll="emit('scroll', ($event.target as HTMLElement).scrollTop)"
  >
    <div class="virtual-canvas" :style="{ height: `${virtualizer.getTotalSize()}px` }">
      <div
        v-for="row in rows"
        :key="row.key.toString()"
        :class="gridRows[row.index]?.label ? 'grid-date-row' : 'media-row'"
        :style="{
          transform: `translateY(${row.start}px)`,
          gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
          height: `${row.size}px`,
        }"
      >
        <span v-if="gridRows[row.index]?.label">{{ gridRows[row.index]?.label }}</span>
        <MediaCard
          v-for="asset in gridRows[row.index]?.items ?? []"
          :key="asset.id"
          :asset="asset"
          :selected="isSelected(asset.id)"
          @select="emit('select', asset, $event)"
          @open="emit('open', asset)"
          @links="emit('links', asset)"
          @favorite="emit('favorite', asset)"
          @context="emit('context', asset, $event)"
        />
      </div>
    </div>
    <div v-if="loadingMore" class="load-more">正在载入更多媒体…</div>
  </div>
</template>
