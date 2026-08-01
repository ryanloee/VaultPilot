// Mock for expo-localization — used by src/i18n/index.ts
export const getLocales = jest.fn(() => [
  { languageCode: 'zh', countryCode: 'CN', languageTag: 'zh-CN' },
]);
export const getCalendars = jest.fn(() => []);
export const isRTL = false;
export default {
  getLocales,
  getCalendars,
  isRTL,
};
