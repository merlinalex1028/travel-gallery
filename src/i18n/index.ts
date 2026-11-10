import { shallowRef } from 'vue';
import zhCN from './zh-CN';
import enUS from './en-US';
export type MessageKey = keyof typeof zhCN;
export type Locale = 'zh-CN' | 'en-US';
export const locale = shallowRef<Locale>('zh-CN');
const catalogs: Record<Locale, Partial<Record<MessageKey, string>>> = { 'zh-CN': zhCN, 'en-US': enUS };
export function setLocale(value: Locale) {
  locale.value = value;
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
