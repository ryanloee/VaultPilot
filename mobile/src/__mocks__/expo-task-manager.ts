/**
 * Jest mock for expo-task-manager.
 * Mirrors the public API consumed by src/services/backgroundSync.ts.
 */

export const _definedTasks = new Map<string, (body: any) => any>();

export function defineTask(taskName: string, task: (body: any) => any): void {
  _definedTasks.set(taskName, task);
}

export function isTaskDefined(taskName: string): boolean {
  return _definedTasks.has(taskName);
}

export async function isTaskDefinedAsync(taskName: string): Promise<boolean> {
  return _definedTasks.has(taskName);
}

export async function unregisterAllTasksAsync(): Promise<void> {
  _definedTasks.clear();
}

export const TaskManagerEvent = { Task: 'Task' };
