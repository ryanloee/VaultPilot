// Mock for expo-sqlite
const mockDb = {
  execAsync: jest.fn().mockResolvedValue(undefined),
  getAllAsync: jest.fn().mockResolvedValue([]),
  getFirstAsync: jest.fn().mockResolvedValue(null),
  runAsync: jest.fn().mockResolvedValue(undefined),
  withTransactionAsync: jest.fn().mockImplementation(async (fn: () => Promise<void>) => fn()),
};

export const openDatabaseAsync = jest.fn().mockResolvedValue(mockDb);

// Helper to get the mock db instance for test configuration
export function __getMockDb() { return mockDb; }

export type SQLiteDatabase = typeof mockDb;
