<script setup lang="ts">
import { locale, tr } from '../i18n';

import { shallowRef } from 'vue';
import { useRouter } from 'vue-router';
import { ElMessageBox } from 'element-plus';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api } from '../services/api';
import type { RelinkPreview, SourceRoot } from '../services/contracts';
import { date } from '../services/format';
import Icon from '../components/Icon.vue';
const props = defineProps<{ source: SourceRoot }>(),
  app = useApp(),
  lib = useLibrary(),
  router = useRouter(),
  relink = shallowRef<RelinkPreview | null>(null),
  relinkOpen = shallowRef(false),
  verifying = shallowRef(false);
async function rescan() {
  await app.perform(async () => {
    await api('rescan_source', { sourceId: props.source.id });
    await app.refresh();
  }, tr('已加入重新扫描任务'));
}
async function enabled() {
  await app.perform(async () => {
    await api('set_source_enabled', { sourceId: props.source.id, enabled: !props.source.isEnabled });
    await app.refresh();
    await lib.search();
  });
}
async function remove() {
  const ok = await ElMessageBox.confirm(
    tr('移除“{0}”的索引会删除其评分、标签关联和整理记录。原始文件不会被删除。建议先禁用来源。', {
      0: props.source.displayName,
    }),
    tr('移除来源索引'),
    { type: 'warning', confirmButtonText: tr('确认移除索引'), cancelButtonText: tr('保留') },
  )
    .then(() => true)
    .catch(() => false);
  if (ok)
    await app.perform(async () => {
      await api('remove_source', { sourceId: props.source.id, confirmed: true });
      await app.refresh();
      await lib.search();
    }, tr('索引已移除，原始文件保留'));
}
async function repair() {
  const selected = await app.perform(() => api('pick_directory', { purpose: 'relink' }));
  if (!selected?.path) return;
  verifying.value = true;
  await app.perform(async () => {
    relink.value = await api('preview_relink_source', {
      sourceId: props.source.id,
      selectedNewPath: selected.path!,
    });
    relinkOpen.value = true;
  });
  verifying.value = false;
}
async function commit() {
  if (!relink.value) return;
  await app.perform(async () => {
    await api('commit_relink_source', {
      sourceId: props.source.id,
      verificationToken: relink.value!.verificationToken,
    });
    relinkOpen.value = false;
    await app.refresh();
  }, tr('来源路径已更新，整理记录已保留'));
}
</script>
<template>
  <article class="source-card">
    <div class="source-card-icon"><Icon name="folder" :size="27" /></div>
    <div class="source-info">
      <div class="row">
        <h3>{{ source.displayName }}</h3>
        <span class="state-pill" :class="source.isOnline ? 'online' : 'warning'"
          ><span class="tiny-dot" :class="source.isOnline ? 'green' : 'amber'" />{{
            source.requiresAuthorization ? tr('需要重新授权') : source.isOnline ? tr('在线') : tr('来源离线')
          }}</span
        ><span v-if="!source.isEnabled" class="state-pill">{{ tr('已禁用') }}</span>
      </div>
      <p class="path-text">{{ source.absolutePath }}</p>
      <div class="source-facts">
        <span>{{ source.totalAssets.toLocaleString(locale) }} {{ tr('个媒体') }}</span
        ><span v-if="source.offlineAssets">{{ source.offlineAssets }} {{ tr('个离线 / 缺失') }}</span
        ><span>{{ tr('上次扫描') }} {{ date(source.lastScanAt, true) }}</span>
      </div>
      <button v-if="source.requiresAuthorization" class="btn small-btn" :disabled="verifying" @click="repair">
        {{ verifying ? tr('正在校验目录…') : tr('重新授权来源') }}
      </button>
    </div>
    <ElDropdown trigger="click"
      ><button class="icon-btn" :aria-label="tr('来源操作')"><Icon name="more" /></button
      ><template #dropdown
        ><ElDropdownMenu
          ><ElDropdownItem
            @click="
              () => {
                lib.setFilter({ sourceIds: [source.id] });
                lib.search();
                router.push('/library');
              }
            "
            >{{ tr('查看此来源照片') }}</ElDropdownItem
          ><ElDropdownItem @click="rescan">{{ tr('重新扫描') }}</ElDropdownItem
          ><ElDropdownItem @click="enabled">{{
            source.isEnabled ? tr('禁用来源（保留记录）') : tr('启用来源')
          }}</ElDropdownItem
          ><ElDropdownItem :disabled="verifying" @click="repair">{{
            verifying ? tr('正在校验新目录…') : tr('重新定位来源')
          }}</ElDropdownItem
          ><ElDropdownItem divided @click="remove">{{ tr('移除来源索引…') }}</ElDropdownItem></ElDropdownMenu
        ></template
      ></ElDropdown
    >
  </article>
  <ElDialog v-model="relinkOpen" :title="tr('确认新的来源位置')" width="640px"
    ><template v-if="relink"
      ><p class="dialog-lead path-text">{{ relink.newPath }}</p>
      <div class="stat-grid">
        <div>
          <strong>{{ relink.matched }}</strong
          ><span>{{ tr('特征匹配') }}</span>
        </div>
        <div>
          <strong>{{ relink.conflicts }}</strong
          ><span>{{ tr('存在冲突') }}</span>
        </div>
        <div>
          <strong>{{ relink.missing }}</strong
          ><span>{{ tr('文件缺失') }}</span>
        </div>
      </div>
      <div class="notice">
        {{ tr('仅在全部特征通过验证后才能提交。原照片不会移动，评分、标签与相册关联会保留。') }}
      </div>
      <div class="relink-samples">
        <div v-for="s in relink.sampleChecks" :key="s.relativePath">
          <Icon :name="s.matched ? 'check' : 'warning'" :size="14" /><span class="truncate">{{
            s.relativePath
          }}</span
          ><span>{{ s.reason }}</span>
        </div>
      </div></template
    ><template #footer
      ><button class="btn" @click="relinkOpen = false">{{ tr('取消') }}</button
      ><button
        class="btn primary"
        :disabled="!relink || !!relink.conflicts || !!relink.missing"
        @click="commit"
      >
        {{ tr('确认更新来源路径') }}
      </button></template
    ></ElDialog
  >
</template>
