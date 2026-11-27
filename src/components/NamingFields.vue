<script setup lang="ts">
import { tr } from '../i18n';

import type { NamingRule } from '../services/contracts';
const model = defineModel<NamingRule>({ required: true });
defineProps<{ disabled?: boolean }>();
function update(field: keyof NamingRule, value: string | number) {
  model.value = { ...model.value, [field]: value };
}
</script>
<template>
  <div class="form-stack naming-fields">
    <label
      >{{ tr('文件名模板')
      }}<input
        :aria-label="tr('文件名模板')"
        :value="model.pattern"
        :disabled="disabled"
        maxlength="300"
        @input="update('pattern', ($event.target as HTMLInputElement).value)"
    /></label>
    <p class="muted small">
      {{
        tr(
          '支持 {date} 日期、{time} 时间、{original} 原名称、{source} 来源、{seq} 序号。保留原扩展名；按文件名排序编号，无拍摄时间显示“未知日期/时间”。',
        )
      }}
    </p>
    <div class="naming-numbers">
      <label
        >{{ tr('起始序号')
        }}<input
          type="number"
          :aria-label="tr('起始序号')"
          :value="model.start"
          :disabled="disabled"
          min="1"
          max="999999999"
          @input="update('start', Number(($event.target as HTMLInputElement).value))"
      /></label>
      <label
        >{{ tr('序号位数')
        }}<input
          type="number"
          :aria-label="tr('序号位数')"
          :value="model.padding"
          :disabled="disabled"
          min="1"
          max="9"
          @input="update('padding', Number(($event.target as HTMLInputElement).value))"
      /></label>
    </div>
  </div>
</template>
