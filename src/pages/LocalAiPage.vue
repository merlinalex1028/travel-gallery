<script setup lang="ts">
import CustomSelect from '../components/CustomSelect.vue';
import { useLocalAi } from '../composables/useLocalAi';
import AiModelPanel from '../components/local-ai/AiModelPanel.vue';
import AiSearchResults from '../components/local-ai/AiSearchResults.vue';
import AiGroupsPanel from '../components/local-ai/AiGroupsPanel.vue';
import Icon from '../components/Icon.vue';
const {
  app,
  trip,
  range,
  status,
  statusError,
  actionError,
  actionBusy,
  starting,
  message,
  query,
  searching,
  results,
  searchedText,
  groups,
  groupCount,
  selectedJob,
  active,
  modelBusy,
  completedGroups,
  canWork,
  maximumGroups,
  refreshStatus,
  refreshGroups,
  toggle,
  importModel,
  removeModel,
  startWork,
  search,
  viewResults,
  viewGroup,
} = useLocalAi();
</script>
<template>
  <section class="standard-page ai-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">DISCOVER CONNECTIONS, ON YOUR DEVICE</div>
        <h1>
          本地 AI <span class="state-pill">{{ status?.enabled ? '已启用' : '已关闭' }}</span>
        </h1>
        <p>用画面内容寻找回忆，把相近的瞬间放在一起。</p>
      </div>
      <button
        class="btn"
        @click="
          refreshStatus();
          refreshGroups();
        "
      >
        <Icon name="refresh" :size="15" />刷新
      </button>
    </div>
    <p v-if="statusError" class="notice warning" role="alert">
      {{ statusError }} <button class="text-btn" @click="refreshStatus">重新读取</button>
    </p>
    <p v-if="actionError" class="notice warning" role="alert">{{ actionError }}</p>
    <p v-if="message" class="notice" role="status">
      {{ message }} <RouterLink to="/jobs">查看任务</RouterLink>
    </p>
    <AiModelPanel
      :status="status"
      :action-busy="actionBusy"
      :active="!!active.length"
      :model-busy="modelBusy"
      @import="importModel"
      @toggle="toggle"
      @remove="removeModel"
    />
    <template v-if="status?.enabled">
      <section class="settings-card ai-work-card">
        <div class="ai-scope">
          <label
            >旅行范围<CustomSelect
              v-model="trip"
              aria-label="AI 旅行范围"
              :options="[
                { value: '', label: '全部已启用来源' },
                ...app.trips.map((item) => ({ value: item.id, label: item.name })),
              ]" /></label
          ><label
            >整理状态<CustomSelect
              v-model="range"
              aria-label="AI 整理范围"
              :options="[
                { value: 'all', label: '全部内容' },
                { value: 'keep', label: '仅保留项' },
              ]"
          /></label>
        </div>
        <div class="ai-index-summary">
          <strong>{{ status.indexedCount }} / {{ status.total }}</strong
          ><span>项已有当前版本的内容索引</span
          ><button class="btn" :disabled="!canWork || status.total > 100000" @click="startWork(false)">
            {{ starting ? '正在提交…' : '建立或更新内容索引' }}
          </button>
        </div>
        <p class="muted small">
          配套文件各自索引。每次任务固定开始时的范围，复用未变化的索引；检测到素材变化后原索引失效。单次索引最多
          100000 项。
        </p>
        <p v-if="active.length" class="notice">
          有本地 AI 任务正在后台处理。<RouterLink to="/jobs">查看进度、暂停或取消</RouterLink>。关闭本地 AI
          会停止索引与分组。
        </p>
        <form class="ai-search-form" @submit.prevent="search">
          <label
            >描述你记得的画面<input
              v-model="query"
              maxlength="200"
              :disabled="searching"
              placeholder="例如：城市街道、山间的湖泊、一座教堂"
              aria-label="语义搜索描述" /></label
          ><button class="btn primary" :disabled="searching || !query.trim() || !status.indexedCount">
            {{ searching ? '正在本机搜索…' : '按内容搜索' }}
          </button>
        </form>
        <p class="muted small">
          搜索仅覆盖已有索引的项目。结果按画面相近程度排列，可能包含不相关内容；分值不是准确率或照片质量。
        </p>
      </section>
      <AiSearchResults :results="results" :searched-text="searchedText" @view="viewResults" />
      <AiGroupsPanel
        v-model:group-count="groupCount"
        v-model:selected-job="selectedJob"
        :groups="groups"
        :completed-groups="completedGroups"
        :trips="app.trips"
        :can-work="canWork"
        :total="status.total"
        :maximum-groups="maximumGroups"
        @generate="startWork(true)"
        @view="viewGroup"
      />
    </template>
  </section>
</template>
<style scoped>
.ai-work-card h2 {
  margin: 0 0 8px;
  font-size: 20px;
}
.ai-scope {
  display: flex;
  gap: 20px;
  flex-wrap: wrap;
}
.ai-scope label {
  display: flex;
  gap: 10px;
  align-items: center;
  white-space: nowrap;
}
.ai-scope .custom-select {
  min-width: 160px;
  max-width: 360px;
  width: auto;
}
.ai-index-summary {
  display: flex;
  gap: 14px;
  align-items: center;
  flex-wrap: wrap;
  margin: 26px 0 12px;
}
.ai-index-summary strong {
  font-size: 25px;
}
.ai-index-summary span {
  color: var(--muted);
}
.ai-index-summary button {
  margin-left: auto;
}
.ai-search-form {
  display: flex;
  gap: 16px;
  align-items: end;
  margin-top: 28px;
}
.ai-search-form label {
  display: grid;
  gap: 10px;
  flex: 1;
}
.ai-search-form button {
  flex-shrink: 0;
}
.ai-page > .settings-card {
  margin-bottom: 24px;
}
@media (max-width: 1100px) {
  .ai-search-form {
    flex-wrap: wrap;
  }
  .ai-search-form label {
    flex-basis: 100%;
  }
}
@media (max-width: 780px) {
  .ai-scope label {
    flex-wrap: wrap;
    max-width: 100%;
  }
  .ai-scope .custom-select {
    min-width: 0;
    max-width: 100%;
  }
}
</style>
