<script setup lang="ts">
import type { AiClusters, JobSummary, Trip } from '../../services/contracts';
import { date } from '../../services/format';
import MediaThumbnail from '../MediaThumbnail.vue';
defineProps<{
  groups: AiClusters | null;
  completedGroups: JobSummary[];
  trips: Trip[];
  canWork: boolean;
  total: number;
  maximumGroups: number;
}>();
const groupCount = defineModel<number>('groupCount', { required: true });
const selectedJob = defineModel<string | null>('selectedJob', { required: true });
const emit = defineEmits<{ generate: []; view: [group: AiClusters['groups'][number]] }>();
</script>
<template>
  <section class="settings-card ai-group-card">
    <div class="row">
      <div>
        <h2>按内容分组</h2>
        <p>使用同一模型的画面特征寻找相近内容，结果由你复核。</p>
      </div>
      <div class="row">
        <label
          >期望组数<input
            v-model.number="groupCount"
            type="number"
            min="2"
            :max="maximumGroups"
            aria-label="内容分组数量" /></label
        ><button
          class="btn"
          :disabled="
            !canWork ||
            total < 2 ||
            total > 10000 ||
            !Number.isInteger(groupCount) ||
            groupCount < 2 ||
            groupCount > maximumGroups
          "
          @click="emit('generate')"
        >
          生成内容组
        </button>
      </div>
    </div>
    <p class="muted small">
      单次最多 10000 项、2–20
      组。会先补齐当前范围索引；画面高度相近时实际组数可能更少。分组与相近程度不改变保留状态、星级、收藏或标签。
    </p>
    <label v-if="completedGroups.length" class="ai-history"
      >查看已生成的分组<select v-model="selectedJob" aria-label="内容分组历史">
        <option :value="null">当前范围最近一次</option>
        <option v-for="job in completedGroups" :key="job.id" :value="job.id">
          {{ date(job.createdAt, true) }} · {{ job.progressDone }} 步
        </option>
      </select></label
    >
    <template v-if="groups?.jobId"
      ><p class="muted small">
        {{ date(groups.createdAt, true) }} ·
        {{
          groups.filter.collectionId
            ? (trips.find((t) => t.id === groups?.filter.collectionId)?.name ?? '原旅行范围')
            : '全部已启用来源'
        }}
        · 当时范围 {{ groups.summary?.scopeCount }} 项，已分组 {{ groups.summary?.clusteredCount }} 项，{{
          groups.summary?.actualGroups
        }}
        组<span v-if="groups.summary?.failedCount"
          >，{{ groups.summary.failedCount }} 项无法处理（见任务明细）</span
        >。
      </p>
      <div class="ai-group-grid">
        <article v-for="group in groups.groups" :key="group.id" class="ai-group">
          <div class="ai-group-previews">
            <MediaThumbnail
              v-for="asset in group.previews"
              :key="asset.id"
              :asset-id="asset.id"
              :alt="asset.filename"
              variant="medium"
            />
          </div>
          <div class="row">
            <h3>{{ group.name }}</h3>
            <strong>{{ group.validCount }} 项有效</strong>
          </div>
          <p v-if="group.staleCount" class="warning-text small">
            {{ group.staleCount }} 项素材或范围已变化，建议更新索引后重新分组。
          </p>
          <button class="btn small-btn" :disabled="!group.validCount" @click="emit('view', group)">
            查看当前有效成员
          </button>
        </article>
      </div></template
    >
    <p v-else class="muted">当前范围尚未生成内容组。</p>
  </section>
</template>
<style scoped>
.ai-group-card h2 {
  margin: 0 0 8px;
  font-size: 20px;
}
.ai-group-card label {
  display: flex;
  gap: 8px;
  align-items: center;
  white-space: nowrap;
}
.ai-group-card input[type='number'] {
  width: 76px;
}
.ai-history {
  margin: 20px 0;
}
.ai-history select {
  max-width: 360px;
}
.ai-group-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 20px;
  margin-top: 20px;
}
.ai-group {
  border: var(--bw) solid var(--line);
  border-radius: var(--r-lg);
  padding: 14px;
}
.ai-group-previews {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 6px;
  margin-bottom: 10px;
}
.ai-group-previews :deep(.thumbnail) {
  height: 110px;
  border-radius: var(--r-sm);
}
.ai-group h3 {
  font-size: 15px;
}
.ai-group strong {
  font-size: 12px;
}
@media (max-width: 1100px) {
  .ai-group-card > .row {
    flex-wrap: wrap;
    gap: 20px;
  }
}
</style>
