import { invoke, isTauri } from '@tauri-apps/api/core';
import type { Commands } from './contracts';
import { errorText, locale, tr } from '../i18n';

export const native = isTauri();
export const isDemo = !native;

export async function api<K extends keyof Commands>(
  name: K,
  request: Commands[K]['request'],
): Promise<Commands[K]['result']> {
  if (!native) {
    const { demoCall } = await import('./demo');
    return demoCall(name, request) as Promise<Commands[K]['result']>;
  }
  if (import.meta.env.MODE === 'performance') {
    const started = performance.now();
    try {
      const result = await invoke<Commands[K]['result']>(name, { request });
      if (name === 'resolve_preview') {
        const input = request as Commands['resolve_preview']['request'];
        const preview = result as Commands['resolve_preview']['result'];
        performance.mark('catalog:previewResolved', {
          detail: {
            assetId: preview.assetId,
            requestedAssetId: input.assetId,
            variant: preview.variant,
            requestedVariant: input.variant,
            resourceUrl: preview.resourceUrl,
            ready: preview.ready,
          },
        });
      }
      return result;
    } finally {
      performance.measure(`catalog:${name}`, { start: started, end: performance.now() });
    }
  }
  return invoke<Commands[K]['result']>(name, { request });
}
export function errorMessage(error: unknown): string {
  if (error && typeof error === 'object') {
    if ('message' in error && typeof error.message === 'string' && error.message.trim()) {
      const message = tr(error.message);
      // Keep unknown native diagnostics intact; a translated code supplies context without hiding detail.
      if (
        locale.value === 'en-US' &&
        message === error.message &&
        /\p{Script=Han}/u.test(message) &&
        'code' in error
      )
        return `${errorText(error.code)} · ${message}`;
      return message;
    }
    if ('code' in error) return errorText(error.code);
  }
  return typeof error === 'string' ? tr(error) : errorText('INTERNAL');
}
export function resourceUrl(url: string | null) {
  if (url?.startsWith('media:') && navigator.userAgent.includes('Windows'))
    return url.replace('media://localhost/', 'http://media.localhost/');
  return url;
}
