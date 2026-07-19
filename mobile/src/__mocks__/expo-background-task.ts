/**
 * Jest mock for expo-background-task.
 * Mirrors the public API consumed by src/services/backgroundSync.ts.
 */

export enum BackgroundTaskStatus {
  Restricted = 1,
  Available = 2,
}

export enum BackgroundTaskResult {
  Success = 1,
  Failed = 2,
}

export const _registeredTasks = new Map<string, any>();

export const registerTaskAsync = jest.fn(async (taskId: string, options?: any) => {
  _registeredTasks.set(taskId, options ?? {});
  return Promise.resolve();
});

export const unregisterTaskAsync = jest.fn(async (taskId: string) => {
  _registeredTasks.delete(taskId);
  return Promise.resolve();
});

export const getStatusAsync = jest.fn(async () => Promise.resolve(BackgroundTaskStatus.Available));
