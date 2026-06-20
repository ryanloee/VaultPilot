// Mock for expo-secure-store
const store: Record<string, string> = {};

export const getItemAsync = jest.fn().mockImplementation(async (key: string) => store[key] ?? null);
export const setItemAsync = jest.fn().mockImplementation(async (key: string, value: string) => { store[key] = value; });
