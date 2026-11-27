<script setup lang="ts">
import { tr } from '../../i18n';

import type { SemanticResults } from '../../services/contracts';
import MediaThumbnail from '../MediaThumbnail.vue';
import EmptyState from '../EmptyState.vue';
defineProps<{ results: SemanticResults | null; searchedText: string }>();
const emit = defineEmits<{ view: [] }>();
</script>
<template>
  <section v-if="results" class="ai-results">
    <div class="row">
      <h2>“{{ searchedText }}{{ tr('”的相近画面') }}</h2>
      <button v-if="results.items.length" class="btn" @click="emit('view')">
        {{ tr('在图库查看本次') }} {{ results.items.length }} {{ tr('项结果') }}
      </button>
    </div>
    <p class="muted small">
      {{ tr('本次范围') }} {{ results.total }} {{ tr('项，其中') }} {{ results.indexedCount }}
      {{ tr('项参与排序；最多显示') }} {{ results.limit }} {{ tr('项。')
      }}<span v-if="results.invalidCount"
        >{{ results.invalidCount }} {{ tr('项索引无效，请重新建立。') }}</span
      >
    </p>
    <div v-if="results.items.length" class="ai-result-grid">
      <figure v-for="item in results.items" :key="item.asset.id" class="ai-result-card">
        <MediaThumbnail :asset-id="item.asset.id" :alt="item.asset.filename" variant="medium" fit="contain" />
        <figcaption>
          <strong>{{ item.asset.filename }}</strong
          ><span v-if="item.asset.mediaKind === 'video'" class="state-pill">{{ tr('视频静帧') }}</span
          ><small :title="tr('余弦相似度，范围 -1 至 1；不是概率')"
            >{{ tr('相近程度') }} {{ item.score.toFixed(3) }}</small
          >
        </figcaption>
      </figure>
    </div>
    <EmptyState
      v-else
      icon="search"
      :title="tr('当前范围还没有可检索的索引')"
      :description="tr('可先建立内容索引，或调整旅行与整理范围。')"
    />
  </section>
</template>
<style scoped>
.ai-results {
  margin: 32px 0;
}
.ai-result-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
  gap: 20px;
}
.ai-result-card {
  margin: 0;
  min-width: 0;
  padding: 12px;
  background: var(--surface);
  border: var(--bw) solid var(--line);
  border-radius: var(--r-lg);
  box-shadow: var(--shadow);
}
.ai-result-card :deep(.thumbnail) {
  height: 210px;
  border-radius: var(--r-sm);
  background: var(--surface-raised);
}
.ai-result-card figcaption {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  padding: 10px 0;
  align-items: center;
  overflow-wrap: anywhere;
}
.ai-result-card strong {
  font-size: 13px;
}
.ai-result-card small {
  display: block;
  width: 100%;
  color: var(--muted);
}
</style>
