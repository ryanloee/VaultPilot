/**
 * Regression tests for background sync (#3158).
 *
 * Covers:
 *   - Default configuration (disabled, 30 min)
 *   - Configuration persistence + clamping to minimum interval
 *   - Task registration / unregistration via configureBackgroundSync
 *   - Task body behaviour: no-config, unreachable backend, successful sync
 *   - applyBackgroundSyncFromConfig round-trip
 *
 * Mocks for expo-background-task / expo-task-manager are provided via
 * moduleNameMapper in jest.config.js (see src/__mocks__/).
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import {
  getBackgroundSyncConfig,
  configureBackgroundSync,
  applyBackgroundSyncFromConfig,
  backgroundSyncTaskBody,
  DEFAULT_CONFIG,
  BG_SYNC_ENABLED_KEY,
  BG_SYNC_INTERVAL_KEY,
  BACKGROUND_SYNC_TASK_ID,
  MIN_INTERVAL_MINUTES,
  intervalLabel,
} from '../../services/backgroundSync';
import * as bgTask from 'expo-background-task';
import * as sync from '../../services/sync';

// Reset AsyncStorage + mock call counts between tests.
beforeEach(async () => {
  await AsyncStorage.clear();
  (bgTask.registerTaskAsync as jest.Mock).mockClear();
  (bgTask.unregisterTaskAsync as jest.Mock).mockClear();
});

describe('getBackgroundSyncConfig — defaults & persistence', () => {
  it('returns disabled + 30min when nothing is stored', async () => {
    const cfg = await getBackgroundSyncConfig();
    expect(cfg.enabled).toBe(false);
    expect(cfg.intervalMinutes).toBe(30);
  });

  it('reads back persisted values', async () => {
    await AsyncStorage.setItem(BG_SYNC_ENABLED_KEY, 'true');
    await AsyncStorage.setItem(BG_SYNC_INTERVAL_KEY, '60');
    const cfg = await getBackgroundSyncConfig();
    expect(cfg.enabled).toBe(true);
    expect(cfg.intervalMinutes).toBe(60);
  });

  it('falls back to default interval for invalid values', async () => {
    await AsyncStorage.setItem(BG_SYNC_ENABLED_KEY, 'false');
    await AsyncStorage.setItem(BG_SYNC_INTERVAL_KEY, '7'); // not 15/30/60
    const cfg = await getBackgroundSyncConfig();
    expect(cfg.intervalMinutes).toBe(DEFAULT_CONFIG.intervalMinutes);
  });
});

describe('configureBackgroundSync — persistence + registration', () => {
  it('persists enabled + interval to AsyncStorage', async () => {
    const cfg = await configureBackgroundSync(true, 15);
    expect(cfg.enabled).toBe(true);
    expect(cfg.intervalMinutes).toBe(15);
    expect(await AsyncStorage.getItem(BG_SYNC_ENABLED_KEY)).toBe('true');
    expect(await AsyncStorage.getItem(BG_SYNC_INTERVAL_KEY)).toBe('15');
  });

  it('clamps below-minimum intervals to MIN_INTERVAL_MINUTES', async () => {
    // @ts-expect-error — deliberately invalid value
    const cfg = await configureBackgroundSync(true, 5);
    expect(cfg.intervalMinutes).toBe(MIN_INTERVAL_MINUTES);
    expect(await AsyncStorage.getItem(BG_SYNC_INTERVAL_KEY)).toBe(String(MIN_INTERVAL_MINUTES));
  });

  it('registers the task when enabled', async () => {
    await configureBackgroundSync(true, 30);
    expect(bgTask.registerTaskAsync).toHaveBeenCalledWith(
      BACKGROUND_SYNC_TASK_ID,
      expect.objectContaining({ minimumInterval: 30 }),
    );
  });

  it('does NOT register when disabled (only unregisters)', async () => {
    await configureBackgroundSync(false, 30);
    expect(bgTask.registerTaskAsync).not.toHaveBeenCalled();
    expect(bgTask.unregisterTaskAsync).toHaveBeenCalledWith(BACKGROUND_SYNC_TASK_ID);
  });

  it('always unregisters before re-registering (idempotent interval change)', async () => {
    await configureBackgroundSync(true, 15);
    (bgTask.unregisterTaskAsync as jest.Mock).mockClear();
    (bgTask.registerTaskAsync as jest.Mock).mockClear();
    await configureBackgroundSync(true, 60);
    expect(bgTask.unregisterTaskAsync).toHaveBeenCalledWith(BACKGROUND_SYNC_TASK_ID);
    expect(bgTask.registerTaskAsync).toHaveBeenCalledWith(
      BACKGROUND_SYNC_TASK_ID,
      expect.objectContaining({ minimumInterval: 60 }),
    );
  });
});

describe('applyBackgroundSyncFromConfig — round-trip', () => {
  it('applies whatever is persisted in AsyncStorage', async () => {
    await AsyncStorage.setItem(BG_SYNC_ENABLED_KEY, 'true');
    await AsyncStorage.setItem(BG_SYNC_INTERVAL_KEY, '15');
    const cfg = await applyBackgroundSyncFromConfig();
    expect(cfg.enabled).toBe(true);
    expect(cfg.intervalMinutes).toBe(15);
    expect(bgTask.registerTaskAsync).toHaveBeenCalledWith(
      BACKGROUND_SYNC_TASK_ID,
      expect.objectContaining({ minimumInterval: 15 }),
    );
  });

  it('does not register when disabled in storage', async () => {
    await AsyncStorage.setItem(BG_SYNC_ENABLED_KEY, 'false');
    const cfg = await applyBackgroundSyncFromConfig();
    expect(cfg.enabled).toBe(false);
    expect(bgTask.registerTaskAsync).not.toHaveBeenCalled();
  });
});

describe('backgroundSyncTaskBody — task execution', () => {
  const getServerConfigSpy = jest.spyOn(sync, 'getServerConfig');
  const pingBackendSpy = jest.spyOn(sync, 'pingBackend');
  const syncNotesSpy = jest.spyOn(sync, 'syncNotesFromServer');

  beforeEach(() => {
    getServerConfigSpy.mockReset();
    pingBackendSpy.mockReset();
    syncNotesSpy.mockReset();
  });

  it('returns Failed (2) when no backend is configured', async () => {
    getServerConfigSpy.mockResolvedValue({ url: '', token: '' });
    const result = await backgroundSyncTaskBody();
    expect(result).toBe(2); // BackgroundTaskResult.Failed
    expect(syncNotesSpy).not.toHaveBeenCalled();
  });

  it('returns Failed (2) when backend is unreachable', async () => {
    getServerConfigSpy.mockResolvedValue({ url: 'http://x', token: '' });
    pingBackendSpy.mockResolvedValue(false);
    const result = await backgroundSyncTaskBody();
    expect(result).toBe(2);
    expect(syncNotesSpy).not.toHaveBeenCalled();
  });

  it('returns Success (1) after a clean sync', async () => {
    getServerConfigSpy.mockResolvedValue({ url: 'http://x', token: 't' });
    pingBackendSpy.mockResolvedValue(true);
    syncNotesSpy.mockResolvedValue({ imported: 1, updated: 0, skipped: 0, errors: 0, duration_ms: 10 });
    const result = await backgroundSyncTaskBody();
    expect(result).toBe(1); // BackgroundTaskResult.Success
    expect(syncNotesSpy).toHaveBeenCalledTimes(1);
  });

  it('returns Failed (2) when syncNotesFromServer throws', async () => {
    getServerConfigSpy.mockResolvedValue({ url: 'http://x', token: 't' });
    pingBackendSpy.mockResolvedValue(true);
    syncNotesSpy.mockRejectedValue(new Error('network down'));
    const result = await backgroundSyncTaskBody();
    expect(result).toBe(2);
  });
});

describe('intervalLabel', () => {
  it('returns Chinese labels', () => {
    expect(intervalLabel(15)).toBe('15 分钟');
    expect(intervalLabel(30)).toBe('30 分钟');
    expect(intervalLabel(60)).toBe('1 小时');
  });
});
