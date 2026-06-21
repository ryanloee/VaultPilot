/**
 * Expo config plugin to fix @react-native-voice/voice build.gradle
 * - Replace deprecated jcenter() with mavenCentral()
 * - Ensure compileSdkVersion is set from rootProject
 */
const { withDangerousMod } = require('@expo/config-plugins');
const fs = require('fs');
const path = require('path');

function withVoiceLibFix(config) {
  return withDangerousMod(config, [
    'android',
    (mod) => {
      const buildGradle = path.join(
        mod.modRequest.platformProjectRoot,
        'node_modules/@react-native-voice/voice/android/build.gradle'
      );

      if (fs.existsSync(buildGradle)) {
        let content = fs.readFileSync(buildGradle, 'utf8');
        // Replace jcenter() with mavenCentral()
        content = content.replace(/jcenter\(\)/g, 'mavenCentral()');
        fs.writeFileSync(buildGradle, content);
      }

      return mod;
    },
  ]);
}

module.exports = withVoiceLibFix;
