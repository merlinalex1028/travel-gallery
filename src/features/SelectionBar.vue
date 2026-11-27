<script setup lang="ts">
import { locale, tr } from '../i18n';

import { computed, shallowRef } from 'vue';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api } from '../services/api';
import { date } from '../services/format';
import Icon from '../components/Icon.vue';
import CustomSelect from '../components/CustomSelect.vue';
const app = useApp(),
  lib = useLibrary(),
  tagOpen = shallowRef(false),
  tripOpen = shallowRef(false),
  tags = shallowRef<string[]>([]),
  trip = shallowRef(''),
  tagMode = shallowRef<'add' | 'remove'>('add');
const tagOptions = computed(() => app.tags.map((tag) => ({ value: tag.id, label: tag.name })));
defineEmits<{ cull: []; compare: [] }>();
async function applyTags() {
  await app.perform(async () => {
    await lib.applyTags(
      tagMode.value === 'add' ? tags.value : [],
      tagMode.value === 'remove' ? tags.value : [],
    );
    await app.refresh();
    tagOpen.value = false;
  }, tr('标签已保存'));
}
async function addToTrip() {
  await app.perform(async () => {
    await api('add_to_trip', { tripId: trip.value, target: lib.target });
    await app.refresh();
    tripOpen.value = false;
  }, tr('已加入旅行相册，原文件未移动'));
}
</script>
<template>
  <div class="selection-context">
    <div>
      <button
        class="text-btn"
        :disabled="lib.queryUnavailable || !lib.items.length"
        @click="lib.selectLoaded"
      >
        <Icon name="square" :size="15" />{{ tr('选择当前已加载')
        }}<span v-if="!lib.queryUnavailable"> {{ lib.items.length }} {{ tr('张') }}</span></button
      ><span v-if="lib.loading" class="muted" role="status">{{ tr('· 正在更新匹配结果…') }}</span
      ><span v-else-if="lib.error" class="muted" role="status">{{
        tr('· 查询未完成，请调整条件或重试')
      }}</span
      ><span v-else class="muted"
        >{{ tr('· 当前查询') }} {{ lib.total.toLocaleString(locale) }} {{ tr('张') }}</span
      >
    </div>
    <button
      class="text-btn"
      :disabled="lib.queryUnavailable || !lib.total"
      @click="app.perform(lib.selectAllMatching)"
    >
      {{ tr('选择全部匹配')
      }}<span v-if="!lib.queryUnavailable"> {{ lib.total.toLocaleString(locale) }} {{ tr('张') }}</span
      ><Icon name="all" :size="14" />
    </button>
  </div>
  <div v-if="lib.count" class="selection-bar">
    <span
      ><Icon name="checked" :size="17" />{{ tr('已选') }}
      <strong>{{ lib.count.toLocaleString(locale) }}</strong> {{ tr('张')
      }}<span v-if="lib.snapshot" class="snapshot-label">{{ tr('冻结快照') }}</span></span
    >
    <div class="selection-actions">
      <button
        class="text-btn"
        :disabled="!!lib.pending"
        @click="app.perform(() => lib.apply({ pickState: 'keep' }))"
      >
        <Icon name="flag" :size="15" />{{ tr('保留') }}</button
      ><button
        class="text-btn"
        :disabled="!!lib.pending"
        @click="app.perform(() => lib.apply({ pickState: 'reject' }))"
      >
        <Icon name="close" :size="15" />{{ tr('淘汰') }}</button
      ><ElDropdown trigger="click" @command="(n: number) => app.perform(() => lib.apply({ rating: n }))"
        ><button class="text-btn">
          <Icon name="star" :size="15" />{{ tr('评分') }}<Icon name="down" :size="12" /></button
        ><template #dropdown
          ><ElDropdownMenu
            ><ElDropdownItem v-for="n in 6" :key="n" :command="n - 1"
              >{{ n - 1 }} {{ tr('星') }}</ElDropdownItem
            ></ElDropdownMenu
          ></template
        ></ElDropdown
      ><button class="text-btn" @click="tagOpen = true">
        <Icon name="tags" :size="15" />{{ tr('标签') }}</button
      ><button class="text-btn" @click="tripOpen = true">
        <Icon name="mountain" :size="15" />{{ tr('加入旅行') }}</button
      ><button class="text-btn" @click="$emit('cull')">
        {{ tr('开始挑片') }}<Icon name="next" :size="14" /></button
      ><button
        class="text-btn"
        :disabled="!!lib.snapshot || lib.count < 2 || lib.count > 4"
        @click="$emit('compare')"
      >
        {{ tr('对比') }}</button
      ><button
        class="text-btn"
        @click="
          app.renameJobId = '';
          app.renameOpen = true;
        "
      >
        {{ tr('重命名预演') }}</button
      ><button
        class="btn primary small-btn"
        @click="
          app.exportDraft = null;
          app.exportOpen = true;
        "
      >
        <Icon name="export" :size="15" />{{ tr('复制导出') }}</button
      ><button class="icon-btn" :aria-label="tr('清除选区')" @click="lib.clearSelection">
        <Icon name="close" :size="17" />
      </button>
    </div>
  </div>
  <div v-if="lib.snapshot" class="snapshot-times small muted" role="status">
    <span v-if="lib.excluded.size"
      >{{ tr('已排除') }} {{ lib.excluded.size.toLocaleString(locale) }} {{ tr('张') }}</span
    >
    <span
      >{{ tr('冻结于') }}
      <time :aria-label="tr('选区冻结时间')" :datetime="new Date(lib.snapshot.createdAt).toISOString()">{{
        date(lib.snapshot.createdAt, true)
      }}</time></span
    >
    <span
      >{{ tr('有效至') }}
      <time :aria-label="tr('选区有效期')" :datetime="new Date(lib.snapshot.expiresAt).toISOString()">{{
        date(lib.snapshot.expiresAt, true)
      }}</time></span
    >
  </div>
  <div
    v-if="lib.snapshot && (lib.queryUnavailable || lib.snapshot.queryFingerprint !== lib.fingerprint)"
    class="notice warning"
  >
    <Icon name="info" :size="16" />{{ lib.queryUnavailable ? tr('查询结果尚未就绪') : tr('筛选已改变')
    }}{{ tr('；本次') }} {{ lib.count }} {{ tr('张的冻结选区保持原范围。重新选择全部匹配才会更新范围。') }}
  </div>
  <ElDialog v-model="tagOpen" :title="tr('批量编辑标签')" width="420px"
    ><p class="dialog-lead">{{ tr('应用到本次选中的') }} {{ lib.count }} {{ tr('张照片。') }}</p>
    <div class="form-stack">
      <CustomSelect
        v-model="tagMode"
        :aria-label="tr('标签操作')"
        :options="[
          { value: 'add', label: tr('添加这些标签') },
          { value: 'remove', label: tr('移除这些标签') },
        ]"
      /><CustomSelect
        v-model="tags"
        multiple
        :placeholder="tr('选择标签')"
        :aria-label="tr('批量选择标签')"
        :options="tagOptions"
      />
    </div>
    <RouterLink to="/tags" class="text-btn" @click="tagOpen = false">{{ tr('创建或管理标签') }}</RouterLink
    ><template #footer
      ><button class="btn" @click="tagOpen = false">{{ tr('取消') }}</button
      ><button class="btn primary" :disabled="!tags.length || !!lib.pending" @click="applyTags">
        {{ tr('保存标签') }}
      </button></template
    ></ElDialog
  ><ElDialog v-model="tripOpen" :title="tr('加入旅行相册')" width="420px"
    ><CustomSelect
      v-model="trip"
      :aria-label="tr('选择旅行相册')"
      :options="[
        { value: '', label: tr('选择旅行相册'), disabled: true },
        ...app.trips.map((t) => ({ value: t.id, label: t.name })),
      ]"
    /><template #footer
      ><button class="btn" @click="tripOpen = false">{{ tr('取消') }}</button
      ><button class="btn primary" :disabled="!trip" @click="addToTrip">
        {{ tr('加入') }} {{ lib.count }} {{ tr('张') }}
      </button></template
    ></ElDialog
  >
</template>
<style scoped>
.snapshot-times {
  display: flex;
  flex-wrap: wrap;
  gap: 8px 20px;
  margin: 8px 0;
}
</style>
