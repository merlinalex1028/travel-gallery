<script setup lang="ts">
import { useRoute, useRouter } from 'vue-router';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import type { AssetFilter } from '../services/contracts';
import Icon from './Icon.vue';
import { t } from '../i18n';
const app = useApp(),
  lib = useLibrary(),
  route = useRoute(),
  router = useRouter();
const navigation = [
  { path: '/library', label: 'nav.library', icon: 'images' },
  { path: '/cull', label: 'nav.cull', icon: 'flag' },
  { path: '/duplicates', label: 'nav.duplicates', icon: 'copy' },
  { path: '/trips', label: 'nav.trips', icon: 'mountain' },
  { path: '/timeline', label: 'nav.timeline', icon: 'calendar' },
  { path: '/map', label: 'nav.map', icon: 'map' },
  { path: '/quality', label: 'nav.quality', icon: 'eye' },
  { path: '/ai', label: 'nav.ai', icon: 'layers' },
] as const;
function library(filter: AssetFilter) {
  lib.setFilter(filter);
  void router.push('/library');
  void lib.search();
}
</script>
<template>
  <aside class="sidebar" :class="{ collapsed: app.navCollapsed }">
    <RouterLink to="/library" class="brand"
      ><img src="/logo.svg" :alt="t('app.name')" width="35" height="35" />
      <div>
        <strong>{{ t('app.name') }}</strong
        ><span>TRAVEL GALLERY</span>
      </div></RouterLink
    >
    <div class="nav-scroll">
      <div class="nav-section-label">{{ t('nav.myLibrary') }}</div>
      <nav :aria-label="t('nav.primary')">
        <button
          v-for="nav in navigation"
          :key="nav.path"
          :aria-label="t(nav.label)"
          :title="t(nav.label)"
          class="nav-item"
          :class="{
            active: route.path === nav.path && (nav.path !== '/library' || !Object.keys(lib.filter).length),
          }"
          @click="nav.path === '/library' ? library({}) : router.push(nav.path)"
        >
          <Icon :name="nav.icon" /><span>{{ t(nav.label) }}</span
          ><span v-if="nav.path === '/library'" class="nav-count">{{ app.total.toLocaleString() }}</span>
        </button>
      </nav>
      <div class="nav-section-label nav-section-with-action">
        <span>{{ t('nav.quickAccess') }}</span
        ><RouterLink to="/filter" class="icon-btn" :aria-label="t('nav.createFilter')"
          ><Icon name="plus" :size="14"
        /></RouterLink>
      </div>
      <button
        class="nav-item"
        :aria-label="t('nav.favorites')"
        :class="{ active: route.path === '/library' && lib.filter.favorite }"
        @click="library({ favorite: true })"
      >
        <Icon name="heart" /><span>{{ t('nav.favorites') }}</span></button
      ><button
        class="nav-item"
        :aria-label="t('nav.kept')"
        :class="{ active: lib.filter.pickStates?.[0] === 'keep' }"
        @click="library({ pickStates: ['keep'] })"
      >
        <Icon name="flag" /><span>{{ t('nav.kept') }}</span
        ><span class="tiny-dot green" /></button
      ><button
        class="nav-item"
        :aria-label="t('nav.unreviewed')"
        :class="{ active: lib.filter.pickStates?.[0] === 'unreviewed' }"
        @click="library({ pickStates: ['unreviewed'] })"
      >
        <Icon name="circle" /><span>{{ t('nav.unreviewed') }}</span></button
      ><button
        v-for="saved in app.saved"
        :key="saved.id"
        class="nav-item"
        @click="
          () => {
            lib.setFilter(saved.filter);
            Object.assign(lib.sort, saved.sort);
            router.push('/filter');
            lib.search();
          }
        "
      >
        <Icon name="filter" /><span class="truncate">{{ saved.name }}</span></button
      ><RouterLink class="nav-item" to="/tags" :aria-label="t('nav.tags')" active-class="active"
        ><Icon name="tags" /><span>{{ t('nav.tags') }}</span></RouterLink
      >
      <div class="nav-section-label nav-section-with-action">
        <span>{{ t('nav.sources') }}</span
        ><button class="icon-btn" :aria-label="t('nav.addSource')" @click="app.importOpen = true">
          <Icon name="plus" :size="14" />
        </button>
      </div>
      <button
        v-for="source in app.sources.slice(0, 5)"
        :key="source.id"
        class="nav-item source-nav"
        @click="library({ sourceIds: [source.id] })"
      >
        <Icon name="folder" :size="17" /><span class="truncate">{{ source.displayName }}</span
        ><span
          class="tiny-dot"
          :class="source.isOnline ? 'green' : 'amber'"
          :title="t(source.isOnline ? 'source.online' : 'source.offline')"
        /></button
      ><RouterLink to="/sources" :aria-label="t('nav.manageSources')" class="nav-item subtle"
        ><Icon name="disk" :size="17" /><span>{{ t('nav.manageSources') }}</span
        ><Icon name="right" :size="13"
      /></RouterLink>
    </div>
    <div class="sidebar-bottom">
      <RouterLink class="nav-item" to="/jobs" :aria-label="t('nav.jobs')" active-class="active"
        ><Icon name="jobs" /><span>{{ t('nav.jobs') }}</span
        ><span v-if="app.running.length" class="nav-count running-count">{{
          app.running.length
        }}</span></RouterLink
      ><RouterLink class="nav-item" to="/settings" :aria-label="t('nav.settings')" active-class="active"
        ><Icon name="settings" /><span>{{ t('nav.settings') }}</span></RouterLink
      >
      <div class="local-promise">
        <Icon name="shield" :size="16" />
        <div>
          <strong>{{ t('nav.deviceOnly') }}</strong
          ><small>{{ t('nav.offlinePromise') }}</small>
        </div>
        <span class="tiny-dot green" />
      </div>
    </div>
  </aside>
</template>
