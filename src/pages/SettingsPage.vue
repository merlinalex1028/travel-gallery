<script setup lang="ts">
import CustomSelect from '../components/CustomSelect.vue';
import { onMounted, shallowRef } from 'vue';
import { ElMessageBox } from 'element-plus';
import { useApp } from '../stores/app';
import { useLibrary } from '../stores/library';
import { api, isDemo } from '../services/api';
import { clearPreviewCache } from '../composables/preview';
import { bytes } from '../services/format';
import Icon from '../components/Icon.vue';
import ThirdPartyNotices from '../components/ThirdPartyNotices.vue';
import { version } from '../../package.json';
const app = useApp(),
  lib = useLibrary(),
  cache = shallowRef({ bytes: 0, count: 0 }),
  dbPath = shallowRef('本地应用专属目录'),
  catalogVersion = shallowRef<number | null>(null),
  saving = shallowRef(false),
  savingDiagnostics = shallowRef(false);
async function save() {
  saving.value = true;
  await app.perform(app.persist, '偏好设置已保存');
  saving.value = false;
}
async function backup() {
  const r = await app.perform(() => api('pick_directory', { purpose: 'backup' }));
  if (!r?.path) return;
  await app.perform(async () => {
    await api('backup_catalog', { destinationPath: r.path! });
    await app.refresh();
  }, '资料库备份任务已提交，不包含源媒体');
}
async function restore() {
  const selected = await app.perform(() => api('pick_backup', undefined));
  if (!selected?.path) return;
  const prepared = await app.perform(() => api('prepare_restore', { selectedBackupPath: selected.path! }));
  if (!prepared) return;
  const ok = await ElMessageBox.confirm(
    `恢复“${selected.path}”中的整理记录？恢复前会自动备份当前资料库；照片源文件不会被替换。请先完成或取消所有后台任务。`,
    '恢复资料库',
    { type: 'warning', confirmButtonText: '备份当前库并恢复', cancelButtonText: '取消' },
  )
    .then(() => true)
    .catch(() => false);
  if (ok)
    await app.perform(async () => {
      await api('restore_catalog', {
        selectedBackupPath: selected.path!,
        confirmToken: prepared.confirmToken,
      });
      clearPreviewCache();
      lib.clearSelection();
      lib.history = [];
      lib.session = null;
      await app.init();
      await lib.search();
      cache.value = await api('cache_stats', undefined);
    }, '已恢复资料库，并保留恢复前的自动备份');
}
async function clear(rebuild = false) {
  const ok = await ElMessageBox.confirm(
    '只清理可再生成的预览缓存。照片、评分、标签和旅行相册不会改变。',
    rebuild ? '清理并重建缩略图' : '清理预览缓存',
    { confirmButtonText: '确认清理', cancelButtonText: '取消' },
  )
    .then(() => true)
    .catch(() => false);
  if (ok)
    await app.perform(async () => {
      await api('clear_cache', undefined);
      clearPreviewCache();
      cache.value = await api('cache_stats', undefined);
      if (rebuild)
        for (const source of app.sources.filter((s) => s.isOnline && s.isEnabled))
          await api('rescan_source', { sourceId: source.id });
      await app.refresh();
    }, '缓存已清理，整理记录保留');
}
async function downloadLog() {
  if (!isDemo) {
    if (savingDiagnostics.value) return;
    savingDiagnostics.value = true;
    try {
      const selected = await app.perform(() => api('pick_directory', { purpose: 'diagnostics' }));
      if (!selected?.path) return;
      await app.perform(
        () => api('export_diagnostics', { destinationPath: selected.path! }),
        '脱敏诊断已保存到所选目录',
      );
    } finally {
      savingDiagnostics.value = false;
    }
    return;
  }
  const data = {
    application: 'Travel Gallery',
    version,
    generatedAt: new Date().toISOString(),
    offlineMode: true,
    sourceCount: app.sources.length,
    assetCount: app.total,
    jobs: app.jobs.map((j) => ({
      type: j.type,
      status: j.status,
      progressDone: j.progressDone,
      progressTotal: j.progressTotal,
      failures: j.failureCount,
    })),
  };
  const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
  const url = URL.createObjectURL(blob),
    a = document.createElement('a');
  a.href = url;
  a.download = 'travel-gallery-diagnostics.json';
  a.click();
  URL.revokeObjectURL(url);
}
onMounted(async () => {
  await app.perform(async () => {
    cache.value = await api('cache_stats', undefined);
    const status = await api('get_app_status', undefined);
    catalogVersion.value = status.catalogVersion;
    if ('databasePath' in status) dbPath.value = String(status.databasePath);
  });
});
</script>
<template>
  <section class="standard-page settings-page">
    <div class="page-heading">
      <div>
        <div class="eyebrow">MAKE YOURSELF AT HOME</div>
        <h1>设置</h1>
        <p>按你的习惯，让整理更轻松。</p>
      </div>
      <button class="btn primary" :disabled="saving" @click="save">
        <Icon name="check" :size="16" />{{ saving ? '保存中…' : '保存偏好' }}
      </button>
    </div>
    <div class="settings-card">
      <h3><Icon name="sun" />外观与浏览</h3>
      <div class="setting-row">
        <div>
          <strong>外观主题</strong>
          <p>适应环境，也适应你的习惯</p>
        </div>
        <CustomSelect
          v-model="app.settings.theme"
          aria-label="外观主题"
          :options="[
            { value: 'system', label: '跟随系统' },
            { value: 'light', label: '浅色' },
            { value: 'dark', label: '深色' },
          ]"
        />
      </div>
      <div class="setting-row">
        <div>
          <strong>界面字体</strong>
          <p>调整文字大小，保留完整操作</p>
        </div>
        <CustomSelect
          v-model="app.settings.fontSize"
          aria-label="界面字体"
          :options="[
            { value: 13, label: '紧凑 · 13px' },
            { value: 14, label: '标准 · 14px' },
            { value: 16, label: '舒适 · 16px' },
          ]"
        />
      </div>
      <div class="setting-row">
        <div>
          <strong>缩略图尺寸</strong>
          <p>{{ app.settings.density }} px，网格随窗口自适应</p>
        </div>
        <input
          v-model.number="app.settings.density"
          type="range"
          min="150"
          max="320"
          aria-label="默认缩略图尺寸"
        />
      </div>
      <div class="setting-row">
        <div>
          <strong>相邻照片预取</strong>
          <p>预取少量预览，减少连续翻图等待</p>
        </div>
        <CustomSelect
          v-model="app.settings.prefetch"
          aria-label="预取数量"
          :options="[
            { value: 0, label: '关闭' },
            { value: 1, label: '1 张' },
            { value: 2, label: '2 张' },
          ]"
        />
      </div>
    </div>
    <div class="settings-card">
      <h3><Icon name="flag" />挑片</h3>
      <div class="setting-row">
        <div>
          <strong>标记后自动前进</strong>
          <p>成功保存标记后，转到下一张</p>
        </div>
        <ElSwitch v-model="app.settings.autoAdvance" aria-label="标记后自动前进" />
      </div>
      <div class="setting-row">
        <div>
          <strong>快捷键速览</strong>
          <p>P 保留 · X 淘汰 · U 待定 · 0–5 评分 · F 收藏</p>
        </div>
        <button class="btn small-btn" @click="app.helpOpen = true">
          <Icon name="keyboard" :size="15" />查看全部
        </button>
      </div>
    </div>
    <div class="settings-card">
      <h3><Icon name="layers" />性能与缓存</h3>
      <div class="setting-row">
        <div>
          <strong>电池供电时暂停后台重型任务</strong>
          <p>暂停后台缩略图与查重，接通电源后自动继续；前台查看、复制和备份可用。</p>
          <p>
            当前电源：{{
              app.power.onBattery === true
                ? '电池供电'
                : app.power.onBattery === false
                  ? '外接电源'
                  : '未知，不自动暂停'
            }}
          </p>
        </div>
        <label class="check-label"
          ><input
            v-model="app.settings.pauseOnBattery"
            type="checkbox"
            aria-label="电池供电时暂停后台重型任务"
          />启用</label
        >
      </div>
      <div class="setting-row">
        <div>
          <strong>后台并发上限</strong>
          <p>移动硬盘建议使用较低并发</p>
        </div>
        <CustomSelect
          v-model="app.settings.scanConcurrency"
          aria-label="后台并发上限"
          :options="[
            ...Array.from({ length: 4 }, (_, index) => {
              const n = index + 1;
              return { value: n, label: String(n) + ' 个任务' };
            }),
          ]"
        />
      </div>
      <div class="setting-row">
        <div>
          <strong>缓存空间上限</strong>
          <p>按最近使用情况淘汰预览，不影响整理记录</p>
        </div>
        <CustomSelect
          v-model="app.settings.cacheLimitGb"
          aria-label="缓存空间上限"
          :options="[
            { value: 0.5, label: '512 MB' },
            { value: 1, label: '1 GB' },
            { value: 5, label: '5 GB' },
            { value: 10, label: '10 GB' },
            { value: 20, label: '20 GB' },
          ]"
        />
      </div>
      <div class="setting-row">
        <div>
          <strong>当前预览缓存</strong>
          <p>{{ bytes(cache.bytes) }} · {{ cache.count }} 个缓存条目</p>
        </div>
        <div class="row">
          <button class="btn small-btn" @click="clear()">清理缓存</button
          ><button class="btn small-btn" @click="clear(true)">清理并重建</button>
        </div>
      </div>
    </div>
    <div class="settings-card">
      <h3><Icon name="disk" />本地资料库</h3>
      <p class="small path-text muted">{{ dbPath }}</p>
      <div class="setting-row">
        <div>
          <strong>备份管理资料</strong>
          <p>仅备份索引、标签、评分、相册和任务，<b>不包含源照片与视频</b></p>
        </div>
        <button class="btn small-btn" @click="backup"><Icon name="download" :size="15" />备份资料库</button>
      </div>
      <div class="setting-row">
        <div>
          <strong>恢复资料库</strong>
          <p>验证备份完整性，恢复前自动备份当前库</p>
        </div>
        <button class="btn small-btn" @click="restore">选择备份恢复…</button>
      </div>
    </div>
    <div class="settings-card privacy-card">
      <h3><Icon name="shield" />隐私与原图保护</h3>
      <p>
        旅图无需账户，不上传照片，不收集遥测。图片、字体、地图和整理算法均在本机运行。淘汰仅改变资料库状态，只有你授权的复制导出会写入目标目录。
      </p>
      <div class="setting-row">
        <div>
          <strong>导出脱敏诊断</strong>
          <p>仅包含版本、任务状态和计数，移除路径、文件名、GPS 与照片内容</p>
        </div>
        <button class="btn small-btn" :disabled="savingDiagnostics" @click="downloadLog">
          {{ savingDiagnostics ? '保存诊断中…' : '导出诊断' }}
        </button>
      </div>
      <div class="about-line">
        <img src="/logo.svg" alt="" width="28" /><strong>旅图 Travel Gallery</strong
        ><span>v{{ version }}</span
        ><span v-if="catalogVersion !== null">资料库格式 {{ catalogVersion }}</span
        ><span v-if="isDemo">浏览器演示资料库</span><ThirdPartyNotices />
      </div>
    </div>
    <div v-if="app.saved.length" class="settings-card">
      <h3><Icon name="filter" />智能筛选</h3>
      <div v-for="s in app.saved" :key="s.id" class="setting-row">
        <strong>{{ s.name }}</strong
        ><button
          class="text-btn"
          @click="
            app.perform(async () => {
              await api('delete_filter', { filterId: s.id });
              await app.refresh();
            })
          "
        >
          删除规则
        </button>
      </div>
    </div>
  </section>
</template>
