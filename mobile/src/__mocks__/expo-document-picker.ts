// Jest mock for expo-document-picker (test environment has no native module).
const canceledResult = { canceled: true, assets: [] };

export const getDocumentAsync = jest.fn(async () => canceledResult);
export const isDocumentInteractionAvailable = jest.fn(() => true);
export const dismissDocumentInteraction = jest.fn();
