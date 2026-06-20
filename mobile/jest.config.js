module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  roots: ['<rootDir>/src'],
  testMatch: ['**/__tests__/**/*.test.ts'],
  moduleNameMapper: {
    '^expo-sqlite$': '<rootDir>/src/__mocks__/expo-sqlite.ts',
    '^@react-native-async-storage/async-storage$': '<rootDir>/src/__mocks__/async-storage.ts',
    '^expo-secure-store$': '<rootDir>/src/__mocks__/expo-secure-store.ts',
  },
};
