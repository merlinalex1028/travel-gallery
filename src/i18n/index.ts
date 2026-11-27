import { shallowRef } from 'vue';
import zhCN from './zh-CN';
import enUS from './en-US';
import englishMessages from './messages.en.json';
export type MessageKey = keyof typeof zhCN;
export type Locale = 'zh-CN' | 'en-US';
const storageKey = 'travel-gallery-language';
function savedLocale(): Locale {
  try {
    return localStorage.getItem(storageKey) === 'en-US' ? 'en-US' : 'zh-CN';
  } catch {
    return 'zh-CN';
  }
}
export const locale = shallowRef<Locale>(savedLocale());
const catalogs: Record<Locale, Partial<Record<MessageKey, string>>> = { 'zh-CN': zhCN, 'en-US': enUS };
export function setLocale(value: Locale) {
  if (value !== 'zh-CN' && value !== 'en-US') return;
  locale.value = value;
  try {
    localStorage.setItem(storageKey, value);
  } catch {
    // Switching also works for this session when browser storage is unavailable.
  }
}
/** Source-message translations for product copy. User content is never passed here. */
export function tr(source: string, params: Record<string, unknown> = {}): string {
  const key = source.replace(/\s+/g, ' ').trim();
  const message =
    locale.value === 'en-US' && Object.hasOwn(englishMessages, key)
      ? (englishMessages as Record<string, string>)[key]!
      : source;
  return message.replace(/\{(\w+)\}/g, (token, name: string) =>
    Object.hasOwn(params, name) ? String(params[name] ?? token) : token,
  );
}
/** Keep labels created once in setup reactive without rewriting user-owned values. */
export function translatedRecord<K extends string>(messages: Record<K, string>): Readonly<Record<K, string>> {
  return new Proxy(messages, {
    get(target, property, receiver) {
      const message = Reflect.get(target, property, receiver);
      return typeof property === 'string' && Object.hasOwn(target, property) ? tr(message) : message;
    },
  });
}
export function t(key: MessageKey, params: Record<string, string | number> = {}): string {
  const message = catalogs[locale.value][key] ?? zhCN[key];
  return message.replace(/\{(\w+)\}/g, (token, name: string) => String(params[name] ?? token));
}
export function errorText(code: unknown): string {
  const key = `error.${String(code)}`;
  return t(Object.hasOwn(zhCN, key) ? (key as MessageKey) : 'error.INTERNAL');
}
/** Getters track locale even when a shared enum map is read in a template. */
export function localizedRecord<K extends string>(keys: Record<K, MessageKey>): Readonly<Record<K, string>> {
  return new Proxy(keys, {
    get(target, property, receiver) {
      const key = Reflect.get(target, property, receiver);
      return typeof property === 'string' && Object.hasOwn(target, property) ? t(key) : key;
    },
  });
}
