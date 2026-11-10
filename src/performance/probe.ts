import { invoke } from '@tauri-apps/api/core';
import { photoKey } from '../composables/keyboard';
import { installNavigationObserver } from './navigation';

// Imported only by Vite's performance mode. The ordinary observer never drives
// the UI or catalog. A separate, explicit canary build may add a labeled control
// for one trusted-click loopback request; its events are recorded independently.
type Observation = Record<string, unknown>;
type Review = { favorite: boolean; pick: string; rating: number };
type PendingKey = {
  id: number;
  key: string;
  at: number;
  filename: string;
  expected: Partial<Review>;
  observed: boolean;
  accepted: boolean;
};
type ClockReply = { runId: string; nativeReceivedMs: number; nativeRespondedMs: number };
export function install() {
  const queue: Observation[] = [];
  const pending: PendingKey[] = [];
  let writing = false;
  let failed = false;
  let inputId = 0;
  let ready = false;
  let imagesReady = false;
  let lastFrame: number | undefined;
  let segment: { id: number; start: number } | undefined;
  let segmentId = 0;
  let automaticScrollSegmentStarted = false;
  let observer: PerformanceObserver | undefined;
  let ipcObserver: PerformanceObserver | undefined;
  const state = () => ({ visibility: document.visibilityState, focused: document.hasFocus() });
  const record = (value: Observation) => {
    if (queue.length < 2048) queue.push({ at: performance.now(), ...state(), ...value });
    else failed = true;
  };
  if (import.meta.env.VITE_TRAVEL_GALLERY_OFFLINE_CANARY === '1')
    void import('./offlineNetwork').then((module) => module.installOfflineNetworkCanary(record));
  const sampleNavigationFrame = installNavigationObserver(record);
  const review = (): Review | null => {
    const root = document.querySelector('.viewer-controls-bar .review-controls');
    if (!root) return null;
    const picks = ['keep', 'maybe', 'reject'];
    const pickIndex = [...root.querySelectorAll('.pick-controls button')].findIndex(
      (button) => button.getAttribute('aria-pressed') === 'true',
    );
    const rating = root.querySelector('.star-btn[aria-pressed="true"]')?.getAttribute('aria-label');
    return {
      favorite: root.querySelector('[aria-label="切换收藏 F"]')?.getAttribute('aria-pressed') === 'true',
      pick: picks[pickIndex] ?? 'unreviewed',
      rating: Number(rating?.match(/\d+/)?.[0] ?? 0),
    };
  };
  const filename = () => document.querySelector('.viewer-photo-bar > span')?.textContent?.trim() ?? '';
  async function flush() {
    if (writing || failed || !queue.length) return;
    writing = true;
    const events = queue.splice(0, 128);
    const sent = performance.now();
    try {
      const reply = await invoke<ClockReply>('performance_probe', { request: { events } });
      // Each clock sample is recorded in the next batch, not recursively flushed.
      record({ type: 'clock', sent, received: performance.now(), ...reply });
    } catch (error) {
      failed = true;
      console.error('Benchmark observer stopped; measurements are incomplete.', error);
    } finally {
      writing = false;
    }
  }
  record({
    type: 'probeInstalled',
    timeOrigin: performance.timeOrigin,
    userAgent: navigator.userAgent,
    longTaskSupported: PerformanceObserver.supportedEntryTypes?.includes('longtask') ?? false,
    method:
      'trusted DOM key delivery → matching rendered attributes → two RAF callbacks; not OS pixel presentation',
  });
  if (PerformanceObserver.supportedEntryTypes?.includes('longtask')) {
    observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries())
        record({ type: 'longTask', startTime: entry.startTime, duration: entry.duration });
    });
    observer.observe({ type: 'longtask', buffered: true });
  }
  if (PerformanceObserver.supportedEntryTypes?.includes('measure')) {
    ipcObserver = new PerformanceObserver((list) => {
      for (const entry of list.getEntries())
        if (entry.name.startsWith('catalog:'))
          record({ type: 'ipcTiming', command: entry.name.slice(8), startTime: entry.startTime, duration: entry.duration });
    });
    ipcObserver.observe({ type: 'measure', buffered: true });
  }
  document.addEventListener('visibilitychange', () => record({ type: 'visibility' }));
  window.addEventListener('focus', () => record({ type: 'focus' }));
  window.addEventListener('blur', () => record({ type: 'blur' }));
  document.addEventListener(
    'keydown',
    (event) => {
      if (!event.isTrusted) return;
      const at = performance.now();
      if (event.key === 'F9' || (event.ctrlKey && event.altKey && event.code === 'Digit9')) {
        if (segment) {
          record({ type: 'segmentEnd', id: segment.id, start: segment.start, reason: 'F9' });
          segment = undefined;
        } else {
          segment = { id: ++segmentId, start: at };
          lastFrame = undefined;
          record({ type: 'segmentStart', ...segment, maximumDurationMs: 60000 });
        }
        return;
      }
      const key = photoKey(event);
      const before = review();
      if (!key || !before || !['f', 'p', 'x', 'u', '0', '1', '2', '3', '4', '5'].includes(key)) return;
      const id = ++inputId;
      const name = filename();
      // Include accepted inputs even when a previous write is still queued.
      const previousFavorite =
        [...pending].reverse().find((item) => item.filename === name && item.expected.favorite !== undefined)
          ?.expected.favorite ?? before.favorite;
      const expected: Partial<Review> =
        key === 'f'
          ? { favorite: !previousFavorite }
          : key === 'p'
            ? { pick: 'keep' }
            : key === 'x'
              ? { pick: 'reject' }
              : key === 'u'
                ? { pick: 'maybe' }
                : { rating: Number(key) };
      const item: PendingKey = { id, key, at, filename: name, expected, observed: false, accepted: false };
      record({
        type: 'keyDelivered',
        id,
        key,
        deliveredAt: at,
        eventTimeStamp: event.timeStamp,
        repeat: event.repeat,
        filename: name,
        before,
        expected,
      });
      const earlierPending = pending.some((earlier) => earlier.filename === name);
      // Reserve during capture: several native events can arrive before timers.
      pending.push(item);
      // WebKit can drain microtasks between capture and bubble listeners.
      // A timer runs after the product's keyboard handler has had its turn.
      setTimeout(() => {
        if (!event.defaultPrevented) {
          record({ type: 'keyNotHandled', id });
          pending.splice(pending.indexOf(item), 1);
          return;
        }
        if (
          !earlierPending &&
          Object.entries(expected).every(([key, value]) => before[key as keyof Review] === value)
        ) {
          record({ type: 'keyAlreadyInState', id });
          pending.splice(pending.indexOf(item), 1);
          return;
        }
        item.accepted = true;
      }, 0);
    },
    true,
  );
  document.addEventListener(
    'scroll',
    (event) => {
      if (event.isTrusted && event.target instanceof HTMLElement && event.target.matches('.media-viewport'))
        record({ type: 'viewportScroll', scrollTop: event.target.scrollTop });
      if (
        event.isTrusted &&
        event.target instanceof Element &&
        event.target.matches('.media-viewport') &&
        !segment &&
        !automaticScrollSegmentStarted
      ) {
        automaticScrollSegmentStarted = true;
        segment = { id: ++segmentId, start: performance.now() };
        lastFrame = undefined;
        record({
          type: 'segmentStart',
          ...segment,
          trigger: 'first viewport scroll',
          maximumDurationMs: 60000,
        });
      }
    },
    true,
  );
  function frame(now: number) {
    sampleNavigationFrame();
    if (segment) {
      if (now - segment.start >= 60000) {
        record({ type: 'segmentEnd', id: segment.id, start: segment.start, reason: '60000ms' });
        segment = undefined;
      } else {
        record({
          type: 'frame',
          segmentId: segment.id,
          timestamp: now,
          intervalMs: lastFrame === undefined ? null : now - lastFrame,
        });
      }
    }
    lastFrame = now;
    if (!ready && document.querySelector('.library-page .media-card')) {
      ready = true;
      record({
        type: 'libraryInteractiveOpportunity',
        frontendAt: now,
        indexedCount: document.querySelector('.heading-count')?.textContent,
      });
      // Flush immediately so its native receipt gives a conservative launch bound.
      void flush();
    }
    if (ready && !imagesReady) {
      const images = [...document.querySelectorAll<HTMLImageElement>('.media-card .thumbnail img')];
      const cards = document.querySelectorAll('.media-card').length;
      if (
        cards &&
        images.length === cards &&
        images.every((image) => image.complete && image.naturalWidth > 0)
      ) {
        imagesReady = true;
        record({ type: 'initialMountedImagesReady', count: images.length });
      }
    }
    if (pending.length) {
      const current = review();
      const name = filename();
      for (const item of [...pending]) {
        if (item.observed || !item.accepted) continue;
        if (now - item.at > 10000) {
          record({ type: 'keyRenderTimeout', id: item.id, filename: item.filename });
          pending.splice(pending.indexOf(item), 1);
        } else if (
          current &&
          name === item.filename &&
          Object.entries(item.expected).every(([key, value]) => current[key as keyof Review] === value)
        ) {
          // Do not assign one visible toggle to multiple waiting F presses.
          if (pending.some((earlier) => earlier.id < item.id && !earlier.observed)) break;
          item.observed = true;
          const firstRaf = performance.now();
          record({ type: 'keyMatchingDomAtRaf', id: item.id, elapsedMs: firstRaf - item.at, current });
          requestAnimationFrame(() => {
            const secondReview = review();
            record({
              type: 'keyPaintOpportunity',
              id: item.id,
              firstRaf,
              elapsedMs: performance.now() - item.at,
              current: secondReview,
              filename: filename(),
              stillMatching:
                filename() === item.filename &&
                !!secondReview &&
                Object.entries(item.expected).every(
                  ([key, value]) => secondReview[key as keyof Review] === value,
                ),
            });
            pending.splice(pending.indexOf(item), 1);
          });
          break;
        }
      }
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
  setInterval(() => {
    for (const item of [...pending]) {
      if (!item.observed && performance.now() - item.at > 10000) {
        record({
          type: 'keyRenderTimeout',
          id: item.id,
          filename: item.filename,
          reason: 'No matching foreground RAF before deadline; never count as feedback success',
        });
        pending.splice(pending.indexOf(item), 1);
      }
    }
    const viewport = document.querySelector<HTMLElement>('.media-viewport');
    record({
      type: 'domSample',
      route: location.hash,
      allElements: document.getElementsByTagName('*').length,
      cards: document.querySelectorAll('.media-card').length,
      rows: document.querySelectorAll('.media-row,.grid-date-row').length,
      scrollTop: viewport?.scrollTop,
      scrollHeight: viewport?.scrollHeight,
      viewportHeight: viewport?.clientHeight,
      indexedCount: document.querySelector('.heading-count')?.textContent,
      queuedKeys: pending.length,
      observerFailed: failed,
    });
  }, 1000);
  setInterval(() => void flush(), 500);
  window.addEventListener('pagehide', () => {
    observer?.disconnect();
    ipcObserver?.disconnect();
    void flush();
  });
}
