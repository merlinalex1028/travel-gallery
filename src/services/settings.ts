import defaults from './default-settings.json';
import type { AppSettings } from './contracts';

/** Fresh effective values; missing fields in an older backup use product defaults. */
export function settingsWithDefaults(overrides: Partial<AppSettings> = {}): AppSettings {
  return { ...defaults, ...overrides } as AppSettings;
}
