<script setup lang="ts">
import { computed } from 'vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { picks } from '../services/format';
import type { AssetFilter } from '../services/contracts';
import Icon from '../components/Icon.vue';
const app = useApp(),
  lib = useLibrary();
defineEmits<{ filter: []; save: []; refresh: [] }>();
const activeCount = computed(
  () =>
    Object.values(lib.filter).filter((v) => v !== undefined && v !== '' && (!Array.isArray(v) || v.length))
      .length,
);
const chips = computed(() =>
  Object.entries(lib.filter)
    .filter(([, v]) => v !== undefined && v !== '' && (!Array.isArray(v) || v.length))
    .map(([key, value]) => ({
      key,
      label:
        key === 'captureDateKnown'
          ? value
            ? '有拍摄日期'
            : '未知拍摄日期'
          : key === 'pickStates'
            ? (lib.filter.pickStates ?? []).map((s) => picks[s]).join(' / ')
            : key === 'favorite'
              ? value
                ? '仅收藏'
                : '排除收藏'
              : key === 'ratingMin'
                ? `${value} 星及以上`
                : key === 'mediaKinds'
                  ? (lib.filter.mediaKinds ?? []).map((s) => (s === 'image' ? '照片' : '视频')).join(' / ')
                  : key === 'sourceIds'
                    ? '指定来源'
                    : key === 'keyword'
                      ? `搜索：${value}`
                      : key === 'collectionId'
                        ? '当前旅行'
                        : ((
                            {
                              extensions: '指定格式',
                              cameraModels: '指定设备',
                              tagRule: '指定标签',
                              fileStates: '文件状态',
                              duplicateKind: '重复分组',
                              captureFromMs: '拍摄开始日期',
                              captureToMs: '拍摄结束日期',
                              captureLocalFrom: '拍摄开始日期',
                              captureLocalTo: '拍摄结束日期',
                              gpsCell: '地图位置',
                              combineLinked: '展开配套文件',
                              selectionSnapshotId: '冻结挑片范围',
                              aiClusterId: 'AI 内容组',
                              minBytes: '最小大小',
                              maxBytes: '最大大小',
                              minWidth: '最低宽度',
                              minHeight: '最低高度',
                              ratingMax: '最高星级',
                              indexedFromMs: '导入开始',
                              indexedToMs: '导入结束',
                              durationMinMs: '最短视频',
                              durationMaxMs: '最长视频',
                              folder: '目录前缀',
                            } as Record<string, string>
                          )[key] ?? key),
    })),
);
function quick(kind: string) {
  lib.filter.mediaKinds = kind ? [kind as 'image' | 'video'] : undefined;
  void lib.search();
}
function remove(key: string) {
  delete lib.filter[key as keyof AssetFilter];
  void lib.search();
}
</script>
<template>
  <div class="library-toolbar">
    <div class="segmented">
      <button :class="{ active: !lib.filter.mediaKinds?.length }" @click="quick('')">全部</button
      ><button :class="{ active: lib.filter.mediaKinds?.[0] === 'image' }" @click="quick('image')">
        照片</button
      ><button :class="{ active: lib.filter.mediaKinds?.[0] === 'video' }" @click="quick('video')">
        视频
      </button>
    </div>
    <div class="toolbar-right">
      <label class="check-label"
        ><input
          type="checkbox"
          :checked="lib.filter.combineLinked !== false"
          @change="
            lib.filter.combineLinked = ($event.target as HTMLInputElement).checked;
            lib.search();
          "
        />合并配套</label
      >
      <button class="btn small-btn" :class="{ tinted: activeCount }" @click="$emit('filter')">
        <Icon name="filter" :size="15" />筛选<span v-if="activeCount" class="filter-count">{{
          activeCount
        }}</span></button
      ><select v-model="lib.sort.field" class="sort-select" aria-label="排序字段" @change="lib.search()">
        <option value="captureUtcMs">拍摄时间</option>
        <option value="indexedAt">导入时间</option>
        <option value="filename">文件名称</option>
        <option value="byteSize">文件大小</option>
        <option value="rating">星级评分</option></select
      ><button
        class="icon-btn"
        :aria-label="lib.sort.direction === 'asc' ? '改为降序' : '改为升序'"
        @click="
          () => {
            lib.sort.direction = lib.sort.direction === 'asc' ? 'desc' : 'asc';
            lib.search();
          }
        "
      >
        <Icon :name="lib.sort.direction === 'asc' ? 'up' : 'down'" :size="17" /></button
      ><span class="toolbar-divider" /><Icon name="grid" :size="15" /><input
        v-model.number="app.settings.density"
        type="range"
        min="150"
        max="320"
        class="density-slider"
        aria-label="缩略图尺寸"
        @change="app.perform(app.persist)"
      /><button
        class="icon-btn"
        :class="{ tinted: app.settings.showInspector }"
        aria-label="显示或隐藏详情栏"
        @click="app.settings.showInspector = !app.settings.showInspector"
      >
        <Icon name="inspector" :size="18" />
      </button>
    </div>
  </div>
  <div v-if="chips.length" class="filter-chips">
    <span v-for="chip in chips" :key="chip.key" class="filter-chip"
      >{{ chip.label
      }}<button :aria-label="`移除${chip.label}筛选`" @click="remove(chip.key)">
        <Icon name="close" :size="12" /></button></span
    ><button
      class="text-btn"
      @click="
        () => {
          lib.setFilter({});
          lib.search();
        }
      "
    >
      清空条件</button
    ><button class="text-btn" @click="$emit('save')"><Icon name="plus" :size="13" />保存为智能筛选</button>
  </div>
</template>
