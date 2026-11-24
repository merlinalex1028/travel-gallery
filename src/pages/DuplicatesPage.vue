<script setup lang="ts">
import CustomSelect from '../components/CustomSelect.vue';
import { computed, onMounted, onUnmounted, shallowRef, watch } from 'vue';
import { ElMessageBox } from 'element-plus';
import { useRoute, useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api, errorMessage, isDemo } from '../services/api';
import type { DuplicateGroup } from '../services/contracts';
import { bytes, picks } from '../services/format';
import Icon from '../components/Icon.vue';
import MediaThumbnail from '../components/MediaThumbnail.vue';
import EmptyState from '../components/EmptyState.vue';
import BurstGroups from '../features/BurstGroups.vue';
const app = useApp(),
  lib = useLibrary(),
  route = useRoute(),
  router = useRouter(),
  kind = shallowRef<'exact' | 'similar' | 'burst'>(
    route.query.kind === 'burst' ? 'burst' : route.query.kind === 'similar' ? 'similar' : 'exact',
  ),
  groups = shallowRef<DuplicateGroup[]>([]),
  total = shallowRef(0),
  cursor = shallowRef<string | null>(null),
  loading = shallowRef(false),
  threshold = shallowRef(8),
  windowSeconds = shallowRef(60),
  source = shallowRef(''),
  onlyUnreviewed = shallowRef(false),
  error = shallowRef(''),
  loadingMembers = shallowRef(new Set<string>()),
  memberErrors = shallowRef<Record<string, string>>({});
const sourceRoots = computed(() => new Map(app.sources.map((s) => [s.id, s.absolutePath])));
const active = computed(() =>
  app.running.some((j) => j.type === (kind.value === 'exact' ? 'HASH_EXACT' : 'HASH_SIMILAR')),
);
const finishedRecompute = computed(
  () =>
    app.jobs.find(
      (job) =>
        job.operation === 'recomputeExact' && ['completed', 'failed', 'cancelled'].includes(job.status),
    )?.id,
);
let loadGeneration = 0;
onUnmounted(() => {
  loadGeneration++;
});
async function load(more = false) {
  if (more && loading.value) return;
  const generation = more ? loadGeneration : ++loadGeneration;
  if (!more) {
    groups.value = [];
    total.value = 0;
    cursor.value = null;
    loadingMembers.value = new Set();
    memberErrors.value = {};
  }
  error.value = '';
  if (kind.value === 'burst') {
    loading.value = false;
    return;
  }
  const requestedKind = kind.value;
  loading.value = true;
  try {
    const r = await api('query_duplicate_groups', {
      kind: requestedKind,
      cursor: more ? cursor.value : null,
      pageSize: 20,
    });
    if (generation !== loadGeneration || requestedKind !== kind.value) return;
    groups.value = more ? [...groups.value, ...r.items] : r.items;
    total.value = r.total;
    cursor.value = r.nextCursor;
    error.value = '';
  } catch (e) {
    if (generation === loadGeneration && requestedKind === kind.value) error.value = errorMessage(e);
  } finally {
    if (generation === loadGeneration) loading.value = false;
  }
}
watch(kind, () => {
  void router.replace({ query: { kind: kind.value } });
  void load();
});
watch([active, finishedRecompute], ([current, finished], [prior, priorFinished]) => {
  if ((prior && !current) || (kind.value === 'exact' && finished && finished !== priorFinished)) void load();
});
onMounted(() => {
  void load();
});
async function start() {
  if (kind.value === 'burst') return;
  const requestedKind = kind.value;
  await app.perform(async () => {
    const snapshot = await api('create_selection_snapshot', {
      filter: {
        ...(source.value ? { sourceIds: [source.value] } : {}),
        ...(onlyUnreviewed.value ? { pickStates: ['unreviewed'] } : {}),
        fileStates: ['available', 'unsupported', 'corrupt'],
      },
      sort: { field: 'captureUtcMs', direction: 'asc' },
    });
    await api('start_duplicate_scan', {
      kind: requestedKind,
      scope: { kind: 'snapshot', snapshotId: snapshot.id },
      similarity: {
        maxHammingDistance: threshold.value,
        timeWindowSeconds: windowSeconds.value,
        algorithmVersion: '1',
      },
    });
    await app.refresh();
  }, '检测已开始，结果会在后台更新');
}
async function representative(g: DuplicateGroup, aid: string) {
  await app.perform(async () => {
    await api('set_group_representative', { groupId: g.id, assetId: aid });
    await load();
  });
}
async function exclude(g: DuplicateGroup, aid: string, excluded: boolean) {
  await app.perform(async () => {
    await api('exclude_group_member', { groupId: g.id, assetId: aid, excluded });
    await load();
  });
}
function otherMembers(g: DuplicateGroup) {
  return g.members.filter((m) => m.asset.id !== g.representativeAssetId && !m.excluded);
}
async function rejectOthers(g: DuplicateGroup) {
  if (!g.representativeAssetId || g.stale) return;
  const members = otherMembers(g);
  if (!members.length) return;
  const ids = members.map((m) => m.asset.id);
  const names = members.map((m) => m.asset.relativePath).join('\n');
  const ok = await ElMessageBox.confirm(
    `将以下 ${ids.length} 张标记淘汰，仅修改资料库：\n${names}`,
    '复核组内淘汰清单',
    { confirmButtonText: '只标记淘汰', cancelButtonText: '取消', type: 'warning' },
  )
    .then(() => true)
    .catch(() => false);
  if (ok)
    await app.perform(async () => {
      await api('set_group_representative', { groupId: g.id, assetId: g.representativeAssetId! });
      await lib.apply({ pickState: 'reject' }, { kind: 'explicit', assetIds: ids });
      await load();
    }, '已标记淘汰，原始文件未改动，可撤销');
}
function exportGroup(g: DuplicateGroup) {
  const ids = g.members.filter((m) => !m.excluded).map((m) => m.asset.id);
  if (!ids.length) return;
  lib.clearSelection();
  lib.selected = new Set(ids);
  app.exportOpen = true;
}
async function loadMembers(group: DuplicateGroup) {
  if (loadingMembers.value.has(group.id) || group.kind !== kind.value) return;
  const generation = loadGeneration,
    requestedKind = kind.value;
  loadingMembers.value = new Set([...loadingMembers.value, group.id]);
  const errors = { ...memberErrors.value };
  delete errors[group.id];
  memberErrors.value = errors;
  try {
    const result = await api('query_group_members', {
      groupId: group.id,
      cursor: group.members.at(-1)?.asset.id ?? null,
      pageSize: 50,
    });
    if (generation !== loadGeneration || requestedKind !== kind.value) return;
    groups.value = groups.value.map((g) => {
      if (g.id !== group.id) return g;
      const seen = new Set(g.members.map((m) => m.asset.id));
      const fresh = result.items.filter((m) => {
        if (seen.has(m.asset.id)) return false;
        seen.add(m.asset.id);
        return true;
      });
      return { ...g, members: [...g.members, ...fresh], totalMembers: result.total };
    });
  } catch (e) {
    if (generation === loadGeneration && requestedKind === kind.value)
      memberErrors.value = { ...memberErrors.value, [group.id]: errorMessage(e) };
  } finally {
    if (generation === loadGeneration)
      loadingMembers.value = new Set([...loadingMembers.value].filter((id) => id !== group.id));
  }
}
</script>
<template>
  <section class="standard-page duplicates-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">LESS REPETITION, MORE MEANING</div>
        <h1>重复与相似</h1>
        <p>找到重复的片段，把决定权留给自己。</p>
      </div>
      <div class="heading-actions">
        <button
          class="btn"
          :disabled="!lib.history.length"
          @click="
            app.perform(async () => {
              await lib.undo();
              await load();
            })
          "
        >
          <Icon name="undo" :size="15" />撤销</button
        ><button v-if="kind !== 'burst'" class="btn primary" :disabled="active || !app.total" @click="start">
          <Icon :name="active ? 'loader' : 'search'" :class="{ spin: active }" :size="16" />{{
            active ? '后台检测中…' : '开始检测'
          }}
        </button>
      </div>
    </div>
    <div class="tab-line">
      <button :class="{ active: kind === 'exact' }" :aria-pressed="kind === 'exact'" @click="kind = 'exact'">
        <Icon name="copy" :size="17" />精确重复<span>SHA-256</span></button
      ><button
        :class="{ active: kind === 'similar' }"
        :aria-pressed="kind === 'similar'"
        @click="kind = 'similar'"
      >
        <Icon name="layers" :size="17" />视觉相似<span>dHash</span></button
      ><button :class="{ active: kind === 'burst' }" :aria-pressed="kind === 'burst'" @click="kind = 'burst'">
        <Icon name="clock" :size="17" />连拍拍摄组<span>按时间</span>
      </button>
    </div>
    <BurstGroups v-if="kind === 'burst'" />
    <template v-else>
      <div class="duplicate-controls">
        <CustomSelect
          v-model="source"
          aria-label="检测来源"
          :options="[
            { value: '', label: '全部已启用来源' },
            ...app.sources.filter((s) => s.isEnabled).map((s) => ({ value: s.id, label: s.displayName })),
          ]"
        /><label class="check-label"><input v-model="onlyUnreviewed" type="checkbox" />仅检测未处理</label
        ><template v-if="kind === 'similar'"
          ><label>最大汉明距离<input v-model.number="threshold" type="number" min="0" max="20" /></label
          ><label
            >时间窗口（秒）<input v-model.number="windowSeconds" type="number" min="0" max="86400" /></label
          ><span class="muted small">0 秒为全库复查</span></template
        >
      </div>
      <div class="notice" :class="{ warning: kind === 'similar' }">
        <Icon :name="kind === 'similar' ? 'info' : 'shield'" />{{
          isDemo
            ? '演示分组仅展示交互。实际查重需要在桌面应用中运行。'
            : kind === 'exact'
              ? '只有文件字节的 SHA-256 一致才会归入精确重复组。不会自动删除任何文件。'
              : '视觉相似只代表候选，汉明距离不是准确率。请逐组人工复核；不会自动判断或删除“较差”的照片。'
        }}
      </div>
      <div class="section-title">
        <strong v-if="!loading && !error"
          >{{ total }} 个{{ kind === 'exact' ? '精确重复' : '视觉相似' }}组</strong
        ><strong v-else>{{ loading ? '正在加载分组…' : '分组加载失败' }}</strong
        ><span v-if="active" class="warning-text small">结果尚未完整</span
        ><button class="text-btn" @click="load()"><Icon name="refresh" :size="14" />刷新</button>
      </div>
      <div v-for="(group, index) in groups" :key="group.id" class="duplicate-group">
        <header>
          <div>
            <strong>第 {{ index + 1 }} 组</strong><span class="muted">{{ group.memberCount }} 个有效成员</span
            ><span v-if="group.kind === 'exact'" class="muted small"
              >全组理论冗余约
              {{
                bytes(Math.max(0, group.memberCount - 1) * (group.members[0]?.asset.byteSize ?? 0))
              }}，仅估算</span
            ><span v-if="group.stale" class="warning-text">{{
              group.kind === 'exact' ? '结果过期，请检查来源和任务中心' : '结果过期，请重算'
            }}</span>
          </div>
          <div>
            <button
              class="btn small-btn"
              :disabled="!group.members.some((m) => !m.excluded)"
              @click="exportGroup(group)"
            >
              复制当前显示文件</button
            ><button
              class="btn small-btn"
              :disabled="!group.representativeAssetId || group.stale || !otherMembers(group).length"
              @click="rejectOthers(group)"
            >
              标记当前显示的其余照片淘汰
            </button>
          </div>
        </header>
        <div class="duplicate-members">
          <article
            v-for="m in group.members"
            :key="m.asset.id"
            :class="{ excluded: m.excluded, representative: group.representativeAssetId === m.asset.id }"
          >
            <button
              class="duplicate-image"
              :aria-label="`查看${m.asset.filename}`"
              @click="router.push(`/viewer/${m.asset.id}`)"
            >
              <MediaThumbnail :asset-id="m.asset.id" :alt="m.asset.filename" variant="medium" /><span
                v-if="group.representativeAssetId === m.asset.id"
                class="representative-badge"
                ><Icon name="flag" :size="12" />代表照片</span
              >
            </button>
            <div class="duplicate-caption">
              <strong>{{ m.asset.filename }}</strong
              ><span
                >{{ m.asset.width ?? '—' }} × {{ m.asset.height ?? '—' }} ·
                {{ bytes(m.asset.byteSize) }}</span
              ><span class="path-text truncate" :title="m.asset.relativePath">{{ m.asset.relativePath }}</span
              ><span class="path-text truncate" :title="sourceRoots.get(m.asset.sourceId)"
                >来源根路径：{{ sourceRoots.get(m.asset.sourceId) ?? '来源信息暂未更新' }}</span
              ><span class="duplicate-review"
                >{{ m.asset.extension.toUpperCase() }} ·
                {{ m.asset.review.rating ? `${m.asset.review.rating} 星` : '未评分' }}</span
              ><span class="duplicate-link">{{
                m.asset.linkedMediaCount
                  ? `${m.asset.linkedMediaType === 'live_photo' ? 'Live Photo' : m.asset.linkedMediaType === 'raw_jpeg' ? 'RAW + JPEG' : '配套文件'} · ${m.asset.linkedMediaCount + 1} 个源文件`
                  : '未确认配套'
              }}</span
              ><span
                >{{ m.asset.captureLocal ?? '未知日期' }} · {{ picks[m.asset.review.pickState]
                }}<template v-if="m.distance !== null"> · 距离 {{ m.distance }}</template></span
              >
              <div class="row">
                <button
                  class="text-btn"
                  :disabled="m.excluded || group.stale"
                  @click="representative(group, m.asset.id)"
                >
                  {{ group.representativeAssetId === m.asset.id ? '已设为代表' : '设为代表' }}</button
                ><button
                  class="text-btn"
                  :disabled="group.stale"
                  @click="exclude(group, m.asset.id, !m.excluded)"
                >
                  {{ m.excluded ? '恢复候选' : '排除误判' }}
                </button>
              </div>
            </div>
          </article>
        </div>
        <div v-if="memberErrors[group.id]" class="notice warning" role="alert">
          {{ memberErrors[group.id] }}
          <button class="text-btn" :disabled="loadingMembers.has(group.id)" @click="loadMembers(group)">
            重试载入
          </button>
        </div>
        <button
          v-if="group.members.length < (group.totalMembers ?? group.memberCount)"
          class="btn small-btn"
          :disabled="loadingMembers.has(group.id)"
          :aria-label="`载入第 ${index + 1} 组更多照片（${group.members.length} / ${group.totalMembers ?? group.memberCount}）`"
          @click="loadMembers(group)"
        >
          <template v-if="loadingMembers.has(group.id)">正在载入组内照片…</template>
          <template v-else
            >载入更多组内照片（{{ group.members.length }} /
            {{ group.totalMembers ?? group.memberCount }}）</template
          >
        </button>
      </div>
      <div v-if="error && groups.length" class="notice warning" role="alert">
        {{ error }}<button class="text-btn" @click="load()">重新加载分组</button>
      </div>
      <EmptyState
        v-if="!groups.length"
        :icon="error ? 'warning' : 'copy'"
        :error="!!error"
        :role="error ? 'alert' : undefined"
        :title="loading ? '正在加载分组…' : error || '还没有检测到分组'"
        description="开始一次检测，或更改来源范围后重试。"
        :action="error ? '重试' : undefined"
        @action="load()"
      /><button v-if="cursor" class="btn" :disabled="loading" @click="load(true)">加载更多分组</button>
    </template>
  </section>
</template>
