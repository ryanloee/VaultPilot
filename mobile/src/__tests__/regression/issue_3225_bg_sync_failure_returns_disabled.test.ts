/**
 * Regression test for #3225: configureBackgroundSync must return
 * enabled:false (and persist that state) when registerTaskAsync throws.
 *
 * Before the fix, the catch block only logged to console and the function
 * returned { enabled: true, ... } — UI showed sync as ON but no OS task
 * was registered, so background sync silently did nothing.
 *
 * After the fix:
 *   - When enabled=true and registerTaskAsync throws, the function
 *     persists 'false' to AsyncStorage and returns { enabled: false, ... }.
 *   - When enabled=false, any error (e.g. unregisterTaskAsync failing)
 *     is still tolerated and returns the requested state (disabled).
 */
import AsyncStorage from '@react-native-async-storage/async-storage';
import {
  configureBackgroundSync,
  BG_SYNC_ENABLED_KEY,
  BG_SYNC_INTERVAL_KEY,
  BACKGROUND_SYNC_TASK_ID,
} from '../../services/backgroundSync';
import * as bgTask from 'expo-background-task';

beforeEach(async () => {
  await AsyncStorage.clear();
  (bgTask.registerTaskAsync as jest.Mock).mockClear();
  (bgTask.unregisterTaskAsync as jest.Mock).mockClear();
  (bgTask.registerTaskAsync as jest.Mock).mockReset();
  (bgTask.unregisterTaskAsync as jest.Mock).mockReset();
  // Restore default successful implementations after reset.
  (bgTask.registerTaskAsync as jest.Mock).mockResolvedValue(undefined);
  (bgTask.unregisterTaskAsync as jest.Mock).mockResolvedValue(undefined);
});

describe('#3225: configureBackgroundSync reflects OS registration failure', () => {
  it('returns enabled:false and persists false when registerTaskAsync rejects', async () => {
    // Simulate OS-level registration failure (e.g., user denied background task permission).
    (bgTask.registerTaskAsync as jest.Mock).mockRejectedValue(
      new Error('Permission denied for background tasks'),
    );

    const cfg = await configureBackgroundSync(true, 30);

    // The return value must reflect OS reality, not the optimistic user request.
    expect(cfg.enabled).toBe(false);
    expect(cfg.intervalMinutes).toBe(30);
    // Persisted state must also be corrected so a subsequent app launch
    // doesn't try to re-register with a stale "enabled" flag.
    expect(await AsyncStorage.getItem(BG_SYNC_ENABLED_KEY)).toBe('false');
    // Interval is still persisted (so when user re-enables, last choice is remembered).
    expect(await AsyncStorage.getItem(BG_SYNC_INTERVAL_KEY)).toBe('30');
  });

  it('still attempts registration (and fails) before reverting', async () => {
    (bgTask.registerTaskAsync as jest.Mock).mockRejectedValue(new Error('OS said no'));

    await configureBackgroundSync(true, 15);

    expect(bgTask.registerTaskAsync).toHaveBeenCalledWith(
      BACKGROUND_SYNC_TASK_ID,
      expect.objectContaining({ minimumInterval: 15 }),
    );
  });

  it('does NOT revert when registration succeeds (positive control)', async () => {
    (bgTask.registerTaskAsync as jest.Mock).mockResolvedValue(undefined);

    const cfg = await configureBackgroundSync(true, 30);

    expect(cfg.enabled).toBe(true);
    expect(await AsyncStorage.getItem(BG_SYNC_ENABLED_KEY)).toBe('true');
  });

  it('tolerates unregisterTaskAsync failure when disabling (no false-positive revert)', async () => {
    // Unregister failing while disabling should NOT cause us to revert.
    (bgTask.unregisterTaskAsync as jest.Mock).mockRejectedValue(new Error('not registered'));

    const cfg = await configureBackgroundSync(false, 30);

    expect(cfg.enabled).toBe(false);
    expect(await AsyncStorage.getItem(BG_SYNC_ENABLED_KEY)).toBe('false');
  });
});
