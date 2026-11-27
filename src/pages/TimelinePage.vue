<script setup lang="ts">
import { locale, tr } from '../i18n';

import CustomSelect from '../components/CustomSelect.vue';
import { computed, onMounted, onUnmounted, shallowRef } from 'vue';
import { useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api, errorMessage } from '../services/api';
import type { TimelineDay } from '../services/contracts';
import Icon from '../components/Icon.vue';
import EmptyState from '../components/EmptyState.vue';
const app = useApp(),
  lib = useLibrary(),
  router = useRouter(),
  days = shallowRef<TimelineDay[]>([]),
  loading = shallowRef(false),
  error = shallowRef('');
const tripId = shallowRef('');
const max = computed(() => days.value.reduce((n, d) => Math.max(n, d.count), 1));
const knownDays = computed(() => days.value.filter((d) => d.date !== null).length);
const unknownCount = computed(() => days.value.find((d) => d.date === null)?.count ?? 0);
let generation = 0;
async function load() {
  const request = ++generation;
  loading.value = true;
  days.value = [];
  error.value = '';
  try {
    const result = await api('timeline_stats', {
      filter: tripId.value ? { collectionId: tripId.value } : {},
    });
    if (request === generation) days.value = result;
  } catch (e) {
    if (request === generation) error.value = errorMessage(e);
  } finally {
    if (request === generation) loading.value = false;
  }
}
function open(day: TimelineDay) {
  lib.setFilter({
    ...(day.date === null
      ? { captureDateKnown: false }
      : { captureDateKnown: true, captureLocalFrom: day.date, captureLocalTo: day.date }),
    ...(tripId.value ? { collectionId: tripId.value } : {}),
  });
  void router.push('/filter?showResults=1');
}
onMounted(() => {
  void load();
});
onUnmounted(() => generation++);
</script>
<template>
  <section class="standard-page timeline-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">THE DAYS THAT STAY WITH YOU</div>
        <h1>{{ tr('时间轴') }}</h1>
        <p>{{ tr('沿着时间，重走那些走过的路。') }}</p>
      </div>
      <button class="btn" :disabled="loading" @click="load">
        <Icon name="refresh" :size="15" />{{ tr('刷新') }}
      </button>
    </div>
    <label class="timeline-range"
      >{{ tr('旅行范围') }}
      <CustomSelect
        v-model="tripId"
        :aria-label="tr('时间轴旅行范围')"
        @change="load"
        :options="[
          { value: '', label: tr('全部媒体') },
          ...app.trips.map((trip) => ({ value: trip.id, label: trip.name })),
        ]"
    /></label>
    <p v-if="error" class="notice" role="alert">{{ error }} {{ tr('· 请点击刷新重试。') }}</p>
    <div v-else class="timeline-summary" :aria-busy="loading">
      <div>
        <strong>{{ knownDays }}</strong
        ><span>{{ tr('有媒体的拍摄日') }}</span>
      </div>
      <div>
        <strong>{{ days.reduce((n, d) => n + d.count, 0).toLocaleString(locale) }}</strong
        ><span>{{ tr('旅途中的片段') }}</span>
      </div>
      <div>
        <strong>{{ days.reduce((n, d) => n + d.reviewed, 0).toLocaleString(locale) }}</strong
        ><span>{{ tr('已完成整理') }}</span>
      </div>
      <div>
        <strong>{{ unknownCount.toLocaleString(locale) }}</strong
        ><span>{{ tr('拍摄日期未知') }}</span>
      </div>
    </div>
    <p class="muted small">
      {{
        tr(
          '按照片或视频的当地拍摄日期分组，包含手动修正。仅有文件修改时间的素材归入未知日期，可在照片详情修正拍摄时间和时区。',
        )
      }}
    </p>
    <div v-if="days.length" class="timeline-list">
      <button v-for="day in days" :key="day.date ?? 'unknown'" class="timeline-day" @click="open(day)">
        <span class="timeline-dot" />
        <div class="timeline-date">
          <strong>{{ day.date ?? tr('未知日期') }}</strong
          ><small v-if="day.date === null">{{ tr('缺少可靠拍摄时间') }}</small
          ><small v-else-if="day.unknownTimezone">{{ day.unknownTimezone }} {{ tr('项时区未知') }}</small
          ><small v-else>{{ tr('时区已知') }}</small>
        </div>
        <div class="timeline-meter">
          <div :style="{ width: `${Math.max(3, (day.count / max) * 100)}%` }" />
        </div>
        <div class="timeline-numbers">
          <strong>{{ day.photos }} {{ tr('照片 ·') }} {{ day.videos }} {{ tr('视频') }}</strong
          ><span>{{ day.reviewed }} {{ tr('项已整理') }}</span>
        </div>
        <Icon name="arrowUpRight" :size="18" />
      </button>
    </div>
    <EmptyState
      v-else-if="!error"
      icon="calendar"
      :title="loading ? tr('正在整理时间轴…') : tr('等待一段旅程')"
      :description="tr('导入照片后，按拍摄日期串起你的回忆。时区未知不会被当成精确 UTC 时间。')"
    />
  </section>
</template>
