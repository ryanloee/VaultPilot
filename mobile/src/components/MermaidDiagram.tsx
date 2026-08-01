/**
 * Mermaid Diagram Renderer (#3683).
 *
 * Renders ```mermaid fenced code blocks as actual SVG charts using
 * mermaid.js loaded via WebView. Previously only showed raw code text
 * in a card (#2805).
 *
 * Architecture:
 * - Loads mermaid.js from CDN inside a WebView
 * - Passes the mermaid source code via postMessage
 * - mermaid.js renders to SVG, auto-sized to content
 * - WebView height adjusts dynamically via onMessage callback
 * - Dark theme supported via `theme` config
 *
 * Fallback: if WebView fails to render (e.g. no network), shows the
 * raw code text in a styled card so the content is always visible.
 */
import React, { useCallback, useState, useRef } from 'react';
import { View, Text, StyleSheet, Dimensions } from 'react-native';
import { WebView, type WebViewMessageEvent } from 'react-native-webview';
import Icon from './Icon';

export interface MermaidDiagramProps {
  /** The raw mermaid source code (content between ``` fences) */
  source: string;
  /** Accent color for the header label */
  accentColor: string;
  /** Text color for fallback raw-text display */
  textColor: string;
  /** Whether the app is in dark mode */
  isDark: boolean;
}

/**
 * Escape a string for safe embedding in an HTML/JS template literal.
 * Prevents script injection from note content.
 */
function escapeForJs(str: string): string {
  return str
    .replace(/\\/g, '\\\\')
    .replace(/`/g, '\\`')
    .replace(/\$/g, '\\$')
    .replace(/'/g, "\\'");
}

/**
 * Build the HTML document that loads mermaid.js and renders the diagram.
 */
function buildMermaidHtml(source: string, isDark: boolean): string {
  const theme = isDark ? 'dark' : 'default';
  const bgColor = isDark ? '#111122' : '#ffffff';
  const escaped = escapeForJs(source);

  return `<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0, maximum-scale=1.0">
  <style>
    body {
      margin: 0;
      padding: 8px;
      background-color: ${bgColor};
      display: flex;
      justify-content: center;
      align-items: center;
      font-family: sans-serif;
    }
    #diagram-container {
      width: 100%;
      overflow-x: auto;
    }
    .mermaid {
      display: flex;
      justify-content: center;
    }
    .error-msg {
      color: ${isDark ? '#ff6b6b' : '#c0392b'};
      font-size: 13px;
      padding: 12px;
      text-align: center;
    }
  </style>
</head>
<body>
  <div id="diagram-container">
    <div class="mermaid" id="mermaid-diagram">${escaped}</div>
  </div>
  <script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script>
  <script>
    (function() {
      try {
        mermaid.initialize({
          startOnLoad: false,
          theme: '${theme}',
          securityLevel: 'strict',
          flowchart: { useMaxWidth: true, htmlLabels: true },
          sequence: { useMaxWidth: true },
          gantt: { useMaxWidth: true },
        });

        var diagramEl = document.getElementById('mermaid-diagram');
        var rawSource = diagramEl.textContent;

        mermaid.run({
          nodes: [diagramEl],
        }).then(function() {
          // Measure the rendered SVG height and notify React Native
          setTimeout(function() {
            var svg = diagramEl.querySelector('svg');
            var height = 300; // default fallback
            if (svg) {
              svg.style.maxWidth = '100%';
              svg.style.height = 'auto';
              var bbox = svg.getBBox ? svg.getBBox() : null;
              if (bbox && bbox.height > 0) {
                height = Math.ceil(bbox.height) + 24;
              }
              var rect = svg.getBoundingClientRect();
              if (rect.height > 0) {
                height = Math.ceil(rect.height) + 24;
              }
            }
            window.ReactNativeWebView.postMessage(JSON.stringify({
              type: 'rendered',
              height: height
            }));
          }, 100);
        }).catch(function(err) {
          window.ReactNativeWebView.postMessage(JSON.stringify({
            type: 'error',
            message: err.message || String(err)
          }));
        });
      } catch(e) {
        window.ReactNativeWebView.postMessage(JSON.stringify({
          type: 'error',
          message: e.message || String(e)
        }));
      }
    })();
  </script>
</body>
</html>`;
}

const MAX_HEIGHT = 600;
const DEFAULT_HEIGHT = 300;

export default function MermaidDiagram({
  source,
  accentColor,
  textColor,
  isDark,
}: MermaidDiagramProps) {
  const screenWidth = Dimensions.get('window').width;
  const [renderHeight, setRenderHeight] = useState(DEFAULT_HEIGHT);
  const [hasError, setHasError] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string>('');
  const renderedRef = useRef(false);

  const handleMessage = useCallback((event: WebViewMessageEvent) => {
    try {
      const data = JSON.parse(event.nativeEvent.data);
      if (data.type === 'rendered') {
        if (!renderedRef.current) {
          renderedRef.current = true;
          setRenderHeight(Math.min(data.height || DEFAULT_HEIGHT, MAX_HEIGHT));
        }
      } else if (data.type === 'error') {
        setHasError(true);
        setErrorMsg(data.message || 'Unknown error');
      }
    } catch {
      // Ignore malformed messages
    }
  }, []);

  const html = buildMermaidHtml(source, isDark);
  const cardBg = isDark ? '#1a1a2e' : '#f0f4ff';
  const bodyBg = isDark ? '#111122' : '#e8edf8';

  return (
    <View
      testID="mermaid-diagram-card"
      style={[styles.card, { borderColor: accentColor, backgroundColor: cardBg }]}
    >
      {/* Header */}
      <View style={styles.header}>
        <Icon name="analytics-outline" size={14} color={accentColor} />
        <Text style={[styles.label, { color: accentColor }]}>Mermaid Diagram</Text>
      </View>

      {/* Body: WebView for rendering, or fallback raw text on error */}
      <View style={[styles.body, { backgroundColor: bodyBg }]}>
        {hasError ? (
          <View style={styles.errorContainer}>
            <Text style={[styles.errorTitle, { color: isDark ? '#ff6b6b' : '#c0392b' }]}>
              Diagram render failed
            </Text>
            {errorMsg ? (
              <Text style={[styles.errorDetail, { color: textColor }]}>
                {errorMsg}
              </Text>
            ) : null}
            <Text style={[styles.fallbackLabel, { color: textColor, opacity: 0.7 }]}>
              Showing source code:
            </Text>
            <Text
              style={{
                color: textColor,
                fontSize: 12,
                fontFamily: 'monospace',
                opacity: isDark ? 0.85 : 1,
              }}
            >
              {source}
            </Text>
          </View>
        ) : (
          <WebView
            testID="mermaid-webview"
            source={{ html }}
            style={[styles.webview, { width: screenWidth - 44, height: renderHeight }]}
            scrollEnabled={false}
            onMessage={handleMessage}
            originWhitelist={['*']}
            javaScriptEnabled={true}
            domStorageEnabled={true}
            injectedJavaScript={undefined}
            onError={(e) => {
              setHasError(true);
              setErrorMsg('WebView error');
            }}
          />
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    borderRadius: 8,
    borderWidth: 1.5,
    marginVertical: 8,
    overflow: 'hidden',
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: 'rgba(128,128,128,0.2)',
  },
  label: {
    fontSize: 12,
    fontWeight: '600',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
  },
  body: {
    borderRadius: 6,
    margin: 8,
    overflow: 'hidden',
  },
  webview: {
    borderRadius: 6,
  },
  errorContainer: {
    padding: 12,
  },
  errorTitle: {
    fontSize: 13,
    fontWeight: '700',
    marginBottom: 4,
  },
  errorDetail: {
    fontSize: 12,
    marginBottom: 8,
    fontFamily: 'monospace',
  },
  fallbackLabel: {
    fontSize: 11,
    marginBottom: 4,
  },
});
