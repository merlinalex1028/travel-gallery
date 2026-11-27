<script setup lang="ts">
import { locale, setLocale } from '../i18n';
defineProps<{ compact?: boolean }>();
</script>

<template>
  <div class="language-switch" :class="{ compact }" role="group" aria-label="语言 / Language">
    <button type="button" lang="zh-CN" :aria-pressed="locale === 'zh-CN'" @click="setLocale('zh-CN')">
      中文
    </button>
    <button type="button" lang="en" :aria-pressed="locale === 'en-US'" @click="setLocale('en-US')">
      {{ compact ? 'EN' : 'English' }}
    </button>
  </div>
</template>

<style scoped>
.language-switch {
  display: inline-flex;
  flex-shrink: 0;
  padding: 3px;
  border: 1px solid var(--line);
  border-radius: var(--r-sm);
  background: var(--surface);
}
.language-switch button {
  padding: 7px 11px;
  border: 0;
  border-radius: var(--r-xs);
  background: transparent;
  color: var(--muted);
  font: inherit;
  font-size: 12px;
  cursor: pointer;
}
.language-switch button[aria-pressed='true'] {
  background: var(--accent-soft);
  color: var(--accent-ink);
  font-weight: 700;
}
.language-switch button:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
.compact button {
  padding: 5px 7px;
}
</style>
