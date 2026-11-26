import { Channel, invoke } from '@tauri-apps/api/core';
import { native } from './api';
import { version } from '../../package.json';

export interface AppUpdate {
  version: string;
  notes: string | null;
}
export interface UpdateProgress {
  downloaded: number;
  total: number | null;
}
export const updater = {
  support: () =>
    native
      ? invoke<{ supported: boolean; version: string }>('update_support')
      : Promise.resolve({ supported: false, version }),
  check: () => invoke<AppUpdate | null>('check_app_update'),
  download(onProgress: (progress: UpdateProgress) => void) {
    const progress = new Channel<UpdateProgress>();
    progress.onmessage = onProgress;
    return invoke<void>('download_app_update', { progress });
  },
  install: () => invoke<void>('install_app_update'),
};
