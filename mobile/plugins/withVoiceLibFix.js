/**
 * Expo config plugin to fix @react-native-voice/voice build.gradle
 * - Replace deprecated jcenter() with mavenCentral()
 * - Replace deprecated compileSdkVersion/targetSdkVersion/minSdkVersion with modern equivalents
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
        // Replace deprecated method-style SDK version setters with property-style
        content = content.replace(
          /compileSdkVersion\s+(.+)/g,
          'compileSdk $1'
        );
        content = content.replace(
          /targetSdkVersion\s+(.+)/g,
          'targetSdk $1'
        );
        content = content.replace(
          /minSdkVersion\s+(\d+)/g,
          'minSdk $1'
        );
        fs.writeFileSync(buildGradle, content);
      }

      return mod;
    },
  ]);
}

module.exports = withVoiceLibFix;
