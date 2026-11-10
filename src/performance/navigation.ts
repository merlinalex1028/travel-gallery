import { photoKey } from '../composables/keyboard';

// Read-only observer for the separately identified benchmark app. Handles come
// from the real preview response; no requests, DOM mutations, or input synthesis.
export function installNavigationObserver(record: (event: Record<string, unknown>) => void) {
  let sequence = 0;
  let observeUntil = 0;
  const markSupported = PerformanceObserver.supportedEntryTypes?.includes('mark') ?? false;
  record({ type: 'navigationProbeInstalled', previewHandleMarksSupported: markSupported });
  if (markSupported) {
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (!['catalog:previewResolved', 'catalog:screenPreviewResolved'].includes(entry.name)) continue;
        const detail = (entry as PerformanceMark).detail;
        record({ type: 'previewResolved', resolvedAt: entry.startTime, variant: 'screen', ...detail });
        performance.clearMarks(entry.name);
      }
    });
    observer.observe({ type: 'mark', buffered: true });
    window.addEventListener('pagehide', () => observer.disconnect(), { once: true });
  }
  document.addEventListener(
    'keydown',
    (event) => {
      if (!event.isTrusted || !document.querySelector('.viewer-page')) return;
      const key = photoKey(event);
      if (key !== 'arrowleft' && key !== 'arrowright') return;
      const id = ++sequence;
      const deliveredAt = performance.now();
      observeUntil = deliveredAt + 15000;
      record({
        type: 'navigationDelivered',
        id,
        key,
        deliveredAt,
        eventTimeStamp: event.timeStamp,
        repeat: event.repeat,
        filename: document.querySelector('.viewer-photo-bar > span')?.textContent?.trim() ?? '',
      });
      setTimeout(() => record({ type: 'navigationHandled', id, accepted: event.defaultPrevented }), 0);
    },
    true,
  );
  return () => {
    if (performance.now() > observeUntil) return;
    const image = document.querySelector<HTMLImageElement>('.viewer-page .media-stage .stage-image');
    record({
      type: 'viewerFrame',
      navigationId: sequence,
      filename: document.querySelector('.viewer-photo-bar > span')?.textContent?.trim() ?? '',
      position: document.querySelector('.viewer-title > span')?.textContent?.trim() ?? '',
      image: image
        ? {
            src: image.currentSrc,
            requestedSrc: image.src,
            alt: image.alt,
            complete: image.complete,
            width: image.naturalWidth,
            height: image.naturalHeight,
          }
        : null,
      placeholder: document.querySelector('.viewer-page .stage-fallback')?.textContent?.trim() ?? '',
    });
  };
}
