<script setup lang="ts">
import { shallowRef } from 'vue';
import { ElMessageBox } from 'element-plus';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { useRouter } from 'vue-router';
import { api, errorMessage } from '../services/api';
import EmptyState from '../components/EmptyState.vue';
import Icon from '../components/Icon.vue';
const app = useApp(),
  lib = useLibrary(),
  router = useRouter(),
  dialog = shallowRef(false),
  mergeOpen = shallowRef(false),
  id = shallowRef<string | undefined>(),
  name = shallowRef(''),
  color = shallowRef('#9068c0'),
  mergeFrom = shallowRef(''),
  mergeTo = shallowRef(''),
  busy = shallowRef(false),
  saveError = shallowRef('');
let draftId: string | undefined,
  savedDraft = '';
function edit(tag?: (typeof app.tags)[number]) {
  if (busy.value) return;
  id.value = tag?.id;
  draftId = tag?.id;
  savedDraft = '';
  saveError.value = '';
  name.value = tag?.name ?? '';
  color.value = tag?.color ?? '#9068c0';
  dialog.value = true;
}
async function save() {
  if (busy.value || !name.value.trim()) return;
  busy.value = true;
  saveError.value = '';
  draftId ??= id.value ?? crypto.randomUUID();
  const request = { id: draftId, name: name.value.trim(), color: color.value },
    payload = JSON.stringify(request);
  await app.perform(async () => {
    try {
      if (savedDraft !== payload) {
        const result = await api('upsert_tag', request);
        id.value = result.tagId;
        savedDraft = payload;
      }
      await app.refresh();
      dialog.value = false;
    } catch (e) {
      saveError.value = `${savedDraft === payload ? '标签已保存，列表刷新失败：' : ''}${errorMessage(e)}`;
      throw e;
    }
  }, '标签已保存');
  busy.value = false;
}
async function remove(tag: (typeof app.tags)[number]) {
  const ok = await ElMessageBox.confirm(
    `删除“${tag.name}”标签及其 ${tag.count} 个关联？照片不会删除。`,
    '删除标签',
    { type: 'warning' },
  )
    .then(() => true)
    .catch(() => false);
  if (ok)
    await app.perform(async () => {
      await api('delete_tag', { tagId: tag.id });
      await app.refresh();
    }, '标签已删除');
}
async function merge() {
  await app.perform(async () => {
    await api('merge_tags', { sourceTagId: mergeFrom.value, targetTagId: mergeTo.value });
    await app.refresh();
    mergeOpen.value = false;
  }, '标签已合并，重复关联已去重');
}
</script>
<template>
  <section class="standard-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">A LITTLE ORDER, A LOT OF MEMORIES</div>
        <h1>
          标签管理<span class="heading-count">{{ app.tags.length }}</span>
        </h1>
        <p>用你自己的语言，为每一帧找到归属。</p>
      </div>
      <div class="heading-actions">
        <button class="btn" :disabled="app.tags.length < 2" @click="mergeOpen = true">
          <Icon name="layers" :size="15" />合并标签</button
        ><button class="btn primary" :disabled="busy" @click="edit()">
          <Icon name="plus" :size="16" />新建标签
        </button>
      </div>
    </div>
    <div v-if="app.tags.length" class="tags-grid">
      <article v-for="tag in app.tags" :key="tag.id" class="tag-card">
        <span
          class="tag-card-icon"
          :style="{ color: tag.color ?? '#9068c0', background: `${tag.color ?? '#9068c0'}16` }"
          ><Icon name="tags" :size="24" /></span
        ><button
          class="tag-card-name"
          @click="
            () => {
              lib.setFilter({ tagRule: { mode: 'any', tagIds: [tag.id] } });
              router.push('/filter');
            }
          "
        >
          <h3>{{ tag.name }}</h3>
          <p>{{ tag.count }} 张照片</p></button
        ><ElDropdown trigger="click"
          ><button class="icon-btn" :aria-label="`管理${tag.name}`"><Icon name="more" /></button
          ><template #dropdown
            ><ElDropdownMenu
              ><ElDropdownItem @click="edit(tag)">重命名 / 更改颜色</ElDropdownItem
              ><ElDropdownItem @click="remove(tag)">删除标签</ElDropdownItem></ElDropdownMenu
            ></template
          ></ElDropdown
        >
      </article>
    </div>
    <EmptyState
      v-else
      icon="tags"
      title="给回忆加一点注脚"
      description="创建标签，然后在图库中为选中照片批量添加。"
      action="新建标签"
      @action="edit()"
    /><ElDialog v-model="dialog" :title="id ? '编辑标签' : '新建标签'" width="420px"
      ><form class="form-stack" @submit.prevent="save">
        <p v-if="saveError" class="notice warning" role="alert">{{ saveError }} · 请再次保存以重试。</p>
        <label
          >标签名称<input
            v-model="name"
            maxlength="100"
            :disabled="busy"
            placeholder="例如：山野、日落、旅途日常"
            autofocus /></label
        ><label class="color-field">标签颜色<input v-model="color" type="color" :disabled="busy" /></label
        ><button type="submit" class="btn primary" :disabled="!name.trim() || busy">
          {{ busy ? '保存中…' : '保存标签' }}
        </button>
      </form></ElDialog
    ><ElDialog v-model="mergeOpen" title="合并标签" width="440px"
      ><p class="dialog-lead">原标签的照片关联会转移到目标标签，重复关联只保留一次。</p>
      <div class="form-stack">
        <label
          >要合并的标签<select v-model="mergeFrom">
            <option value="">请选择</option>
            <option v-for="t in app.tags" :key="t.id" :value="t.id">{{ t.name }}</option>
          </select></label
        ><label
          >合并到<select v-model="mergeTo">
            <option value="">请选择</option>
            <option v-for="t in app.tags.filter((t) => t.id !== mergeFrom)" :key="t.id" :value="t.id">
              {{ t.name }}
            </option>
          </select></label
        >
      </div>
      <template #footer
        ><button class="btn" @click="mergeOpen = false">取消</button
        ><button
          class="btn primary"
          :disabled="!mergeFrom || !mergeTo || mergeFrom === mergeTo"
          @click="merge"
        >
          确认合并
        </button></template
      ></ElDialog
    >
  </section>
</template>
