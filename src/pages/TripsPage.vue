<script setup lang="ts">
import { tr } from '../i18n';

import { shallowRef } from 'vue';
import { ElMessageBox } from 'element-plus';
import { useApp } from '../stores/app';
import { api, errorMessage } from '../services/api';
import { date } from '../services/format';
import type { Trip } from '../services/contracts';
import Icon from '../components/Icon.vue';
import MediaThumbnail from '../components/MediaThumbnail.vue';
import EmptyState from '../components/EmptyState.vue';
const app = useApp(),
  open = shallowRef(false),
  id = shallowRef<string | undefined>(),
  name = shallowRef(''),
  description = shallowRef(''),
  start = shallowRef(''),
  end = shallowRef(''),
  busy = shallowRef(false),
  saveError = shallowRef('');
let draftId: string | undefined,
  savedDraft = '';
function edit(trip?: Trip) {
  if (busy.value) return;
  id.value = trip?.id;
  draftId = trip?.id;
  savedDraft = '';
  saveError.value = '';
  name.value = trip?.name ?? '';
  description.value = trip?.description ?? '';
  start.value = trip?.startAt ? new Date(trip.startAt).toISOString().slice(0, 10) : '';
  end.value = trip?.endAt ? new Date(trip.endAt).toISOString().slice(0, 10) : '';
  open.value = true;
}
async function save() {
  if (busy.value || !name.value.trim()) return;
  busy.value = true;
  saveError.value = '';
  await app.perform(async () => {
    let payload = '';
    try {
      if (start.value && end.value && start.value > end.value)
        throw { message: tr('结束日期不能早于开始日期') };
      draftId ??= id.value ?? crypto.randomUUID();
      const request = {
        id: draftId,
        name: name.value.trim(),
        description: description.value,
        startAt: start.value ? Date.parse(`${start.value}T00:00:00Z`) : null,
        endAt: end.value ? Date.parse(`${end.value}T23:59:59Z`) : null,
      };
      payload = JSON.stringify(request);
      if (savedDraft !== payload) {
        const result = await api('save_trip', request);
        id.value = result.tripId;
        savedDraft = payload;
      }
      await app.refresh();
      open.value = false;
    } catch (e) {
      saveError.value = `${payload && savedDraft === payload ? tr('旅行已保存，列表刷新失败：') : ''}${errorMessage(e)}`;
      throw e;
    }
  }, tr('旅行相册已保存'));
  busy.value = false;
}
async function remove(trip: Trip) {
  const ok = await ElMessageBox.confirm(
    tr('删除“{0}”相册？仅删除相册与关联，照片和整理记录不变。', { 0: trip.name }),
    tr('删除旅行相册'),
  )
    .then(() => true)
    .catch(() => false);
  if (ok)
    await app.perform(async () => {
      await api('delete_trip', { tripId: trip.id });
      await app.refresh();
    }, tr('旅行相册已删除'));
}
</script>
<template>
  <section class="standard-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">COLLECT MOMENTS, NOT MILES</div>
        <h1>
          {{ tr('旅行相册') }}<span class="heading-count">{{ app.trips.length }}</span>
        </h1>
        <p>{{ tr('每一次出发，都值得被好好收藏。') }}</p>
      </div>
      <button class="btn primary" :disabled="busy" @click="edit()">
        <Icon name="plus" :size="16" />{{ tr('新建旅行') }}
      </button>
    </div>
    <div v-if="app.trips.length" class="trip-grid">
      <article v-for="trip in app.trips" :key="trip.id" class="trip-card">
        <RouterLink :to="`/trips/${trip.id}`" class="trip-cover"
          ><MediaThumbnail
            v-if="trip.coverAssetId"
            :asset-id="trip.coverAssetId"
            :alt="trip.name"
            variant="medium"
          />
          <div v-else class="trip-empty-cover"><Icon name="mountain" :size="50" /></div>
          <span class="trip-count">{{ trip.count }} {{ tr('张回忆') }}</span></RouterLink
        >
        <div class="trip-info">
          <div class="row">
            <RouterLink :to="`/trips/${trip.id}`"
              ><h2>{{ trip.name }}</h2></RouterLink
            ><ElDropdown trigger="click"
              ><button class="icon-btn" :aria-label="tr('旅行相册操作')"><Icon name="more" /></button
              ><template #dropdown
                ><ElDropdownMenu
                  ><ElDropdownItem @click="$router.push(`/trips/${trip.id}/story`)">{{
                    tr('查看故事')
                  }}</ElDropdownItem
                  ><ElDropdownItem @click="edit(trip)">{{ tr('编辑旅行') }}</ElDropdownItem
                  ><ElDropdownItem @click="remove(trip)">{{ tr('删除相册') }}</ElDropdownItem></ElDropdownMenu
                ></template
              ></ElDropdown
            >
          </div>
          <p>{{ trip.description || tr('旅途中的点点滴滴。') }}</p>
          <span class="muted small"
            ><Icon name="calendar" :size="13" />{{ date(trip.startAt) }} — {{ date(trip.endAt) }}</span
          >
        </div>
      </article>
      <button class="new-trip-card" :disabled="busy" @click="edit()">
        <span><Icon name="plus" :size="27" /></span><strong>{{ tr('记录下一次出发') }}</strong
        ><small>{{ tr('创建一个新的旅行相册') }}</small>
      </button>
    </div>
    <EmptyState
      v-else
      icon="mountain"
      :title="tr('下一段故事，从这里开始')"
      :description="tr('旅行相册可以汇集多个来源的照片，无需复制文件。')"
      :action="tr('新建旅行')"
      @action="edit()"
    />
    <p class="muted small">
      {{ tr('在图库中选择照片，点击「加入旅行」归档。相册仅建立数据库关联，不复制源文件。') }}
    </p>
    <ElDialog v-model="open" :title="id ? tr('编辑旅行') : tr('新建旅行相册')" width="480px"
      ><form class="form-stack" @submit.prevent="save">
        <p v-if="saveError" class="notice warning" role="alert">
          {{ saveError }} {{ tr('· 请再次保存以重试。') }}
        </p>
        <label
          >{{ tr('旅行名称')
          }}<input
            v-model="name"
            maxlength="100"
            :placeholder="tr('例如：风从川西来')"
            required
            :disabled="busy" /></label
        ><label
          >{{ tr('关于这次旅行')
          }}<textarea
            v-model="description"
            rows="3"
            :placeholder="tr('记录出发的心情…')"
            :disabled="busy"
          /></label
        ><label
          >{{ tr('旅行日期') }}
          <div class="input-pair">
            <input v-model="start" type="date" :aria-label="tr('旅行开始日期')" :disabled="busy" /><input
              v-model="end"
              type="date"
              :aria-label="tr('旅行结束日期')"
              :disabled="busy"
            /></div></label
        ><button class="btn primary" :disabled="!name.trim() || busy">
          {{ busy ? tr('保存中…') : tr('保存旅行') }}
        </button>
      </form></ElDialog
    >
  </section>
</template>
