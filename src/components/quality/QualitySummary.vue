<script setup lang="ts">
import { tr, translatedRecord } from '../../i18n';

import type { AssetQuality, QualityObservation } from '../../services/contracts';
defineProps<{ analysis: AssetQuality; busy?: boolean }>();
const emit = defineEmits<{ dismiss: [observation: QualityObservation] }>();
const labels = translatedRecord({
  possibleClosed: '可能闭眼',
  possibleBlink: '检查眼睛开合',
  noHint: '未发现明显闭眼提示',
  uncertain: '无法可靠判断',
});
const reasons = translatedRecord({
  smallFace: '人脸或眼睛过小',
  poseOrCrop: '侧脸、裁切或几何不可靠',
  bothEyes: '双眼开合比与模型系数同时触发',
  oneEye: '两眼开合不对称',
  noStrongHint: '未触发当前检查规则',
});
</script>
<template>
  <div class="quality-summary">
    <p v-if="analysis.staleCount" class="warning-text small">
      {{ analysis.staleCount }} {{ tr('项旧分析已失效，请重新扫描并分析。') }}
    </p>
    <section
      v-for="observation in analysis.observations"
      :key="observation.kind"
      :class="{ dismissed: observation.dismissed }"
    >
      <template v-if="observation.kind === 'eyes'">
        <p v-if="!observation.value.faces.length" class="muted small">
          {{ tr('未检出可分析的人脸，无法判断眼睛睁闭。') }}
        </p>
        <ol v-else class="face-list">
          <li v-for="(face, index) in observation.value.faces" :key="index">
            <strong
              :class="{ 'warning-text': face.status === 'possibleClosed' || face.status === 'possibleBlink' }"
              >{{ labels[face.status] }}</strong
            >
            <small
              >{{ reasons[face.reason]
              }}<span v-if="observation.value.faces.length > 1">
                {{ tr('· 人脸') }} {{ index + 1 }}</span
              ></small
            >
          </li>
        </ol>
        <p v-if="observation.value.faceLimitReached" class="warning-text small">
          {{ tr('已达到每张 10 张人脸的分析上限，可能仍有未处理人脸。') }}
        </p>
      </template>
      <p
        v-else
        class="detail-index"
        :title="tr('log10(1 + 固定尺寸预览的拉普拉斯方差)，仅适合相近场景对照；不是质量概率。')"
      >
        <strong
          >{{ tr('细节指数') }} {{ Math.log10(1 + observation.value.laplacianVariance).toFixed(3) }}</strong
        >
        <small>{{ tr('按最长边 512 像素测量；噪点、纹理与构图均会影响数值。') }}</small>
      </p>
      <div class="observation-actions">
        <span v-if="observation.dismissed" class="muted small">{{ tr('已人工忽略') }}</span
        ><button class="text-btn" :disabled="busy" @click="emit('dismiss', observation)">
          {{ observation.dismissed ? tr('恢复显示此记录') : tr('忽略此记录') }}
        </button>
      </div>
    </section>
  </div>
</template>
<style scoped>
.quality-summary section {
  margin-top: 12px;
}
.face-list {
  padding-left: 20px;
  margin: 10px 0;
}
.face-list li {
  margin: 8px 0;
}
.quality-summary small {
  display: block;
  color: var(--muted);
  font-size: 11px;
  line-height: 1.7;
}
.quality-summary strong {
  font-size: 13px;
  font-weight: 600;
}
.detail-index {
  margin: 8px 0;
}
.observation-actions {
  display: flex;
  gap: 10px;
  align-items: center;
  font-size: 12px;
}
.dismissed {
  opacity: 0.7;
}
</style>
