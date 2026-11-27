import { tr } from '../i18n';
import { shallowRef, watch, type MaybeRefOrGetter, toValue } from 'vue';
import { api, resourceUrl } from '../services/api';
import type { PreviewHandle } from '../services/contracts';
type Variant = 'small' | 'medium' | 'screen' | 'poster';
type Lease = { promise: Promise<PreviewHandle | null>; cancel: () => void };
type Task = {
  key: string;
  cacheKey: string;
  epoch: number;
  assetId: string;
  variant: Variant;
  order: number;
  started: boolean;
  consumers: Map<number, number>;
  promise: Promise<PreviewHandle | null>;
  resolve: (value: PreviewHandle | null) => void;
  reject: (error: unknown) => void;
};
const cache = new Map<string, { handle: PreviewHandle; expiresAt: number }>(),
  tasks = new Map<string, Task>(),
  prefetches = new Map<number, Lease>(),
  epoch = shallowRef(0);
let active = 0,
  activeLarge = 0,
  activePrefetch = 0,
  sequence = 0,
  consumer = 0;
function priority(task: Task) {
  return Math.max(...task.consumers.values());
}
function drain() {
  while (active < 4) {
    const task = [...tasks.values()]
      .filter((t) => !t.started && t.consumers.size)
      // A cached small image must not wait behind four long decodes. Keep one
      // slot for small previews, and another large slot available to visible work.
      .filter((t) => (t.variant === 'small' || activeLarge < 3) && (priority(t) > 0 || activePrefetch < 2))
      .sort((a, b) => priority(b) - priority(a) || a.order - b.order)[0];
    if (!task) return;
    task.started = true;
    const large = task.variant !== 'small',
      prefetch = priority(task) <= 0;
    active++;
    if (large) activeLarge++;
    if (prefetch) activePrefetch++;
    void Promise.resolve()
      .then(() =>
        task.consumers.size
          ? api('resolve_preview', {
              assetId: task.assetId,
              variant: task.variant,
              priority: prefetch ? 'prefetch' : 'foreground',
            })
          : null,
      )
      .then((handle) => {
        if (handle?.ready && task.epoch === epoch.value) {
          if (cache.size >= 1200) cache.delete(cache.keys().next().value!);
          cache.set(task.cacheKey, { handle, expiresAt: Date.now() + 600000 });
        }
        task.resolve(handle);
      }, task.reject)
      .finally(() => {
        active--;
        if (large) activeLarge--;
        if (prefetch) activePrefetch--;
        tasks.delete(task.key);
        drain();
      });
  }
}
function acquire(assetId: string, variant: Variant, rank: number): Lease {
  const cacheKey = `${assetId}:${variant}`,
    cached = cache.get(cacheKey);
  if (cached && cached.expiresAt > Date.now())
    return { promise: Promise.resolve(cached.handle), cancel() {} };
  cache.delete(cacheKey);
  const key = `${epoch.value}:${cacheKey}`;
  let task = tasks.get(key);
  if (!task) {
    let resolve!: Task['resolve'], reject!: Task['reject'];
    const promise = new Promise<PreviewHandle | null>((yes, no) => {
      resolve = yes;
      reject = no;
    });
    task = {
      key,
      cacheKey,
      epoch: epoch.value,
      assetId,
      variant,
      order: sequence++,
      started: false,
      consumers: new Map(),
      promise,
      resolve,
      reject,
    };
    tasks.set(key, task);
  }
  const id = consumer++;
  task.consumers.set(id, rank);
  drain();
  return {
    promise: task.promise,
    cancel() {
      task.consumers.delete(id);
      if (!task.started && !task.consumers.size) {
        tasks.delete(key);
        task.resolve(null);
      }
    },
  };
}
export function clearPreviewCache() {
  cache.clear();
  for (const lease of prefetches.values()) lease.cancel();
  prefetches.clear();
  epoch.value++;
}
/** Prefetch stays below visible work and retains only four recent neighbor requests. */
export function prefetchPreview(assetId: string, variant: Variant = 'screen') {
  const id = consumer++,
    lease = acquire(assetId, variant, 0);
  prefetches.set(id, lease);
  while (prefetches.size > 4) {
    const oldest = prefetches.keys().next().value!;
    prefetches.get(oldest)?.cancel();
    prefetches.delete(oldest);
  }
  void lease.promise
    .catch(() => {})
    .finally(() => {
      lease.cancel();
      prefetches.delete(id);
    });
}
export function usePreview(id: MaybeRefOrGetter<string | undefined>, variant: Variant = 'small') {
  const url = shallowRef<string | null>(null),
    reason = shallowRef(''),
    loading = shallowRef(false),
    retryTick = shallowRef(0);
  let retries = 0;
  function retry() {
    const assetId = toValue(id);
    if (!assetId || retries++ >= 1) {
      url.value = null;
      reason.value = tr('预览加载失败，可重新加载或使用系统应用打开');
      return;
    }
    cache.delete(`${assetId}:${variant}`);
    retryTick.value++;
  }
  /** A deliberate user action starts a new attempt; image error retries stay bounded. */
  function reload() {
    const assetId = toValue(id);
    if (!assetId) return;
    retries = 0;
    cache.delete(`${assetId}:${variant}`);
    retryTick.value++;
  }
  watch(
    [() => toValue(id), epoch, retryTick],
    async ([assetId, version], previous, onCleanup) => {
      let stale = false;
      if (assetId !== previous[0] || version !== previous[1]) retries = 0;
      url.value = null;
      reason.value = '';
      loading.value = false;
      if (!assetId) return;
      const lease = acquire(assetId, variant, variant === 'screen' ? 10 : variant === 'medium' ? 7 : 5);
      onCleanup(() => {
        stale = true;
        lease.cancel();
      });
      loading.value = true;
      try {
        const result = await lease.promise;
        if (stale || !result) return;
        url.value = resourceUrl(result.resourceUrl);
        reason.value = result.reason ?? '';
      } catch {
        if (!stale) reason.value = tr('预览不可用，可重试或用系统应用打开');
      } finally {
        if (!stale) loading.value = false;
      }
    },
    { immediate: true, flush: 'sync' },
  );
  return { url, reason, loading, retry, reload };
}
