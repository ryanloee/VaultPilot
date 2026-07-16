// Jest mock for expo-image-picker (test environment has no native module).
const noPermission = { status: 'granted', granted: true, expires: 'never', canAskAgain: true };
const canceledResult = { canceled: true, assets: [] };

export const requestMediaLibraryPermissionsAsync = jest.fn(async () => noPermission);
export const requestCameraPermissionsAsync = jest.fn(async () => noPermission);
export const launchImageLibraryAsync = jest.fn(async () => canceledResult);
export const launchCameraAsync = jest.fn(async () => canceledResult);
export const MediaTypeOptions = { All: 'All', Images: 'Images', Videos: 'Videos' };
export const VideoQuality = { Low: 'low', Medium: 'medium', High: 'high' };
