// Mock for @react-native-async-storage/async-storage
const store: Record<string, string> = {};

export default {
  getItem: jest.fn().mockImplementation(async (key: string) => store[key] ?? null),
  setItem: jest.fn().mockImplementation(async (key: string, value: string) => { store[key] = value; }),
  removeItem: jest.fn().mockImplementation(async (key: string) => { delete store[key]; }),
  clear: jest.fn().mockImplementation(async () => { Object.keys(store).forEach(k => delete store[k]); }),
};
