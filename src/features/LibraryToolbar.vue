<script setup lang="ts">
import { tr } from '../i18n';

import CustomSelect from '../components/CustomSelect.vue';
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
            ? tr('有拍摄日期')
            : tr('未知拍摄日期')
          : key === 'pickStates'
            ? (lib.filter.pickStates ?? []).map((s) => picks[s]).join(' / ')
            : key === 'favorite'
              ? value
                ? tr('仅收藏')
                : tr('排除收藏')
              : key === 'ratingMin'
                ? tr('{0} 星及以上', { 0: value })
                : key === 'mediaKinds'
                  ? (lib.filter.mediaKinds ?? [])
                      .map((s) => (s === 'image' ? tr('照片') : tr('视频')))
                      .join(' / ')
                  : key === 'sourceIds'
                    ? tr('指定来源')
                    : key === 'keyword'
                      ? tr('搜索：{0}', { 0: value })
                      : key === 'collectionId'
                        ? tr('当前旅行')
                        : ((
                            {
                              extensions: tr('指定格式'),
                              cameraModels: tr('指定设备'),
                              tagRule: tr('指定标签'),
                              fileStates: tr('文件状态'),
                              duplicateKind: tr('重复分组'),
                              captureFromMs: tr('拍摄开始日期'),
                              captureToMs: tr('拍摄结束日期'),
                              captureLocalFrom: tr('拍摄开始日期'),
                              captureLocalTo: tr('拍摄结束日期'),
                              gpsCell: tr('地图位置'),
                              combineLinked: tr('展开配套文件'),
                              selectionSnapshotId: tr('冻结挑片范围'),
                              aiClusterId: tr('AI 内容组'),
                              minBytes: tr('最小大小'),
                              maxBytes: tr('最大大小'),
                              minWidth: tr('最低宽度'),
                              minHeight: tr('最低高度'),
                              ratingMax: tr('最高星级'),
                              indexedFromMs: tr('导入开始'),
                              indexedToMs: tr('导入结束'),
                              durationMinMs: tr('最短视频'),
                              durationMaxMs: tr('最长视频'),
                              folder: tr('目录前缀'),
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
      <button :class="{ active: !lib.filter.mediaKinds?.length }" @click="quick('')">{{ tr('全部') }}</button
      ><button :class="{ active: lib.filter.mediaKinds?.[0] === 'image' }" @click="quick('image')">
        {{ tr('照片') }}</button
      ><button :class="{ active: lib.filter.mediaKinds?.[0] === 'video' }" @click="quick('video')">
        {{ tr('视频') }}
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
        />{{ tr('合并配套') }}</label
      >
      <button class="btn small-btn" :class="{ tinted: activeCount }" @click="$emit('filter')">
        <Icon name="filter" :size="15" />{{ tr('筛选')
        }}<span v-if="activeCount" class="filter-count">{{ activeCount }}</span></button
      ><CustomSelect
        v-model="lib.sort.field"
        class="sort-select"
        :aria-label="tr('排序字段')"
        @change="lib.search()"
        :options="[
          { value: 'captureUtcMs', label: tr('拍摄时间') },
          { value: 'indexedAt', label: tr('导入时间') },
          { value: 'filename', label: tr('文件名称') },
          { value: 'byteSize', label: tr('文件大小') },
          { value: 'rating', label: tr('星级评分') },
        ]"
      /><button
        class="icon-btn"
        :aria-label="lib.sort.direction === 'asc' ? tr('改为降序') : tr('改为升序')"
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
        :aria-label="tr('缩略图尺寸')"
        @change="app.perform(app.persist)"
      /><button
        class="icon-btn"
        :class="{ tinted: app.settings.showInspector }"
        :aria-label="tr('显示或隐藏详情栏')"
        @click="app.settings.showInspector = !app.settings.showInspector"
      >
        <Icon name="inspector" :size="18" />
      </button>
    </div>
  </div>
  <div v-if="chips.length" class="filter-chips">
    <span v-for="chip in chips" :key="chip.key" class="filter-chip"
      >{{ chip.label
      }}<button :aria-label="tr('移除{0}筛选', { 0: chip.label })" @click="remove(chip.key)">
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
      {{ tr('清空条件') }}</button
    ><button class="text-btn" @click="$emit('save')">
      <Icon name="plus" :size="13" />{{ tr('保存为智能筛选') }}
    </button>
  </div>
</template>
