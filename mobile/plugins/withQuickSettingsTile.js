/**
 * Expo config plugin for Android Quick Settings Tile (#893).
 * - Registers VaultPilotTileService in AndroidManifest
 * - Adds vaultpilot:// deep link intent filter to MainActivity
 * - Writes the Kotlin TileService source file
 */
const { withAndroidManifest, withDangerousMod } = require('@expo/config-plugins');
const fs = require('fs');
const path = require('path');

const TILE_SERVICE_CLASS = 'com.vaultpilot.mobile.VaultPilotTileService';

function withTileManifest(config) {
  return withAndroidManifest(config, (mod) => {
    const manifest = mod.modResults.manifest;
    if (!manifest.application) manifest.application = [{}];
    const app = manifest.application[0];

    // Add TileService
    if (!app.service) app.service = [];
    const alreadyRegistered = app.service.some(
      (s) => s.$?.['android:name'] === TILE_SERVICE_CLASS
    );
    if (!alreadyRegistered) {
      app.service.push({
        $: {
          'android:name': TILE_SERVICE_CLASS,
          'android:label': 'VaultPilot 快速笔记',
          'android:icon': '@mipmap/ic_launcher',
          'android:permission': 'android.permission.BIND_QUICK_SETTINGS_TILE',
          'android:exported': 'true',
        },
        'intent-filter': [
          { action: [{ $: { 'android:name': 'android.service.quicksettings.action.QS_TILE' } }] },
        ],
      });
    }

    // Add vaultpilot:// deep link to MainActivity
    if (app.activity?.[0]) {
      const activity = app.activity[0];
      if (!activity['intent-filter']) activity['intent-filter'] = [];
      const hasDeepLink = activity['intent-filter'].some(
        (f) => f.data?.some((d) => d.$?.['android:scheme'] === 'vaultpilot')
      );
      if (!hasDeepLink) {
        activity['intent-filter'].push({
          action: [{ $: { 'android:name': 'android.intent.action.VIEW' } }],
          category: [
            { $: { 'android:name': 'android.intent.category.DEFAULT' } },
            { $: { 'android:name': 'android.intent.category.BROWSABLE' } },
          ],
          data: [{ $: { 'android:scheme': 'vaultpilot' } }],
        });
      }
    }

    return mod;
  });
}

function withTileSource(config) {
  return withDangerousMod(config, [
    'android',
    (mod) => {
      const kotlinDir = path.join(
        mod.modRequest.platformProjectRoot,
        'app/src/main/java/com/vaultpilot/mobile'
      );

      const tileSource = `package com.vaultpilot.mobile

import android.content.Intent
import android.net.Uri
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

/**
 * Quick Settings Tile — opens app for quick note creation via deep link.
 * Issue #893
 */
class VaultPilotTileService : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        qsTile?.state = Tile.STATE_ACTIVE
        qsTile?.updateTile()
    }

    override fun onClick() {
        super.onClick()
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse("vaultpilot://note/new")).apply {
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        startActivityAndCollapse(intent)
    }
}
`;

      fs.writeFileSync(path.join(kotlinDir, 'VaultPilotTileService.kt'), tileSource);
      return mod;
    },
  ]);
}

module.exports = function withQuickSettingsTile(config) {
  config = withTileManifest(config);
  config = withTileSource(config);
  return config;
};
