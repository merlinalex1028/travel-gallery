import type { AssetFilter } from './contracts';

export const TEMPORARY_FILTER_MESSAGE = '请移除临时范围，再保存智能筛选。';

export function temporaryFilterField(filter: AssetFilter): 'selectionSnapshotId' | 'aiClusterId' | undefined {
  if (filter.selectionSnapshotId != null) return 'selectionSnapshotId';
  if (filter.aiClusterId != null) return 'aiClusterId';
}
