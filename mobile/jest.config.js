module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  roots: ['<rootDir>/src'],
  testMatch: ['**/__tests__/**/*.test.ts'],
  moduleNameMapper: {
    '^expo-sqlite$': '<rootDir>/src/__mocks__/expo-sqlite.ts',
    '^@react-native-async-storage/async-storage$': '<rootDir>/src/__mocks__/async-storage.ts',
    '^expo-secure-store$': '<rootDir>/src/__mocks__/expo-secure-store.ts',
    '^expo-speech-recognition$': '<rootDir>/src/__mocks__/expo-speech-recognition.ts',
    '^expo-file-system$': '<rootDir>/src/__mocks__/expo-file-system.ts',
    '^expo-file-system/legacy$': '<rootDir>/src/__mocks__/expo-file-system-legacy.ts',
    '^expo-intent-launcher$': '<rootDir>/src/__mocks__/expo-intent-launcher.ts',
    '^react-native$': '<rootDir>/src/__mocks__/react-native.ts',
    '^@expo/vector-icons/Ionicons$': '<rootDir>/src/__mocks__/expo-vector-icons.js',
    '^@expo/vector-icons$': '<rootDir>/src/__mocks__/expo-vector-icons.js',
  },
};
