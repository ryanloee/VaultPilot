/**
 * Expo config plugin for Android Home Screen Widget (#892).
 * - Registers VaultPilotWidgetProvider in AndroidManifest
 * - Writes widget layout XML, widget info XML, and Kotlin provider
 */
const { withAndroidManifest, withDangerousMod } = require('@expo/config-plugins');
const fs = require('fs');
const path = require('path');

const WIDGET_PROVIDER_CLASS = 'com.vaultpilot.mobile.VaultPilotWidgetProvider';

function withWidgetManifest(config) {
  return withAndroidManifest(config, (mod) => {
    const manifest = mod.modResults.manifest;
    if (!manifest.application) manifest.application = [{}];
    const app = manifest.application[0];

    if (!app.receiver) app.receiver = [];
    const alreadyRegistered = app.receiver.some(
      (r) => r.$?.['android:name'] === WIDGET_PROVIDER_CLASS
    );

    if (!alreadyRegistered) {
      app.receiver.push({
        $: {
          'android:name': WIDGET_PROVIDER_CLASS,
          'android:label': 'VaultPilot',
          'android:exported': 'true',
        },
        'intent-filter': [
          { action: [{ $: { 'android:name': 'android.appwidget.action.APPWIDGET_UPDATE' } }] },
        ],
        'meta-data': [{
          $: {
            'android:name': 'android.appwidget.provider',
            'android:resource': '@xml/vaultpilot_widget_info',
          },
        }],
      });
    }

    return mod;
  });
}

function withWidgetSource(config) {
  return withDangerousMod(config, [
    'android',
    (mod) => {
      const projectRoot = mod.modRequest.platformProjectRoot;
      const kotlinDir = path.join(projectRoot, 'app/src/main/java/com/vaultpilot/mobile');
      const layoutDir = path.join(projectRoot, 'app/src/main/res/layout');
      const xmlDir = path.join(projectRoot, 'app/src/main/res/xml');

      // Ensure dirs exist
      fs.mkdirSync(layoutDir, { recursive: true });
      fs.mkdirSync(xmlDir, { recursive: true });

      // Widget layout
      const layoutXml = `<?xml version="1.0" encoding="utf-8"?>
<LinearLayout xmlns:android="http://schemas.android.com/apk/res/android"
    android:layout_width="match_parent"
    android:layout_height="match_parent"
    android:orientation="horizontal"
    android:gravity="center_vertical"
    android:padding="12dp"
    android:background="#1E3A5F">

    <TextView
        android:id="@+id/widget_title"
        android:layout_width="0dp"
        android:layout_height="wrap_content"
        android:layout_weight="1"
        android:text="VaultPilot"
        android:textColor="#FFFFFF"
        android:textSize="16sp"
        android:textStyle="bold" />

    <LinearLayout
        android:layout_width="wrap_content"
        android:layout_height="wrap_content"
        android:orientation="horizontal">

        <TextView
            android:id="@+id/btn_new_note"
            android:layout_width="40dp"
            android:layout_height="40dp"
            android:gravity="center"
            android:text="📝"
            android:textSize="20sp"
            android:background="?android:attr/selectableItemBackgroundBorderless" />

        <TextView
            android:id="@+id/btn_new_chat"
            android:layout_width="40dp"
            android:layout_height="40dp"
            android:gravity="center"
            android:text="💬"
            android:textSize="20sp"
            android:background="?android:attr/selectableItemBackgroundBorderless" />
    </LinearLayout>
</LinearLayout>
`;

      // Widget info
      const widgetInfoXml = `<?xml version="1.0" encoding="utf-8"?>
<appwidget-provider xmlns:android="http://schemas.android.com/apk/res/android"
    android:minWidth="250dp"
    android:minHeight="40dp"
    android:targetCellWidth="2"
    android:targetCellHeight="1"
    android:updatePeriodMillis="0"
    android:initialLayout="@layout/vaultpilot_widget"
    android:resizeMode="horizontal"
    android:widgetCategory="home_screen" />
`;

      // Kotlin provider
      const providerSource = `package com.vaultpilot.mobile

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.widget.RemoteViews

/**
 * Home screen widget — quick new note / new chat actions.
 * Issue #892
 */
class VaultPilotWidgetProvider : AppWidgetProvider() {

    override fun onUpdate(
        context: Context,
        appWidgetManager: AppWidgetManager,
        appWidgetIds: IntArray
    ) {
        for (widgetId in appWidgetIds) {
            val views = RemoteViews(context.packageName, R.layout.vaultpilot_widget)

            // New note button → vaultpilot://note/new
            val noteIntent = Intent(Intent.ACTION_VIEW, Uri.parse("vaultpilot://note/new")).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            }
            val notePending = PendingIntent.getActivity(
                context, 0, noteIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )
            views.setOnClickPendingIntent(R.id.btn_new_note, notePending)

            // New chat button → vaultpilot://chat/new
            val chatIntent = Intent(Intent.ACTION_VIEW, Uri.parse("vaultpilot://chat/new")).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            }
            val chatPending = PendingIntent.getActivity(
                context, 1, chatIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )
            views.setOnClickPendingIntent(R.id.btn_new_chat, chatPending)

            appWidgetManager.updateAppWidget(widgetId, views)
        }
    }
}
`;

      fs.writeFileSync(path.join(layoutDir, 'vaultpilot_widget.xml'), layoutXml);
      fs.writeFileSync(path.join(xmlDir, 'vaultpilot_widget_info.xml'), widgetInfoXml);
      fs.writeFileSync(path.join(kotlinDir, 'VaultPilotWidgetProvider.kt'), providerSource);

      return mod;
    },
  ]);
}

module.exports = function withDesktopWidget(config) {
  config = withWidgetManifest(config);
  config = withWidgetSource(config);
  return config;
};
