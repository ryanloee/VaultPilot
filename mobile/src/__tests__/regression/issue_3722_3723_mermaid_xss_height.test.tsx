// @ts-nocheck
/**
 * Regression tests for MermaidDiagram (#3722, #3723).
 *
 * #3722 — XSS: mermaid source was injected into the WebView HTML as raw text
 *   without HTML-escaping. `<img src=x onerror="alert(1)">` in a note body was
 *   parsed by the browser as a real <img> tag, executing arbitrary JS inside
 *   the WebView before mermaid.js even loaded.
 *   Fix: escapeHtml() replaces escapeForJs(), HTML-escaping all of & < > " '.
 *
 * #3723 — height stuck: renderedRef guard accepted only the FIRST height
 *   message from the WebView. When source/isDark changed the WebView reloaded
 *   and posted a new height, but renderedRef.current was already true so
 *   setRenderHeight was never called again — the diagram was permanently
 *   clipped at the first render's height.
 *   Fix: useEffect resets renderedRef.current = false on source/isDark change.
 */

import React from 'react';
import { render, act } from '@testing-library/react-native';

// --- Mocks ------------------------------------------------------------------

// Capture the props passed to WebView so we can inspect source.html and call onMessage.
let lastWebViewProps: any = null;
jest.mock('react-native-webview', () => {
  const R = require('react');
  const { View } = require('react-native');
  return {
    WebView: (props: any) => {
      lastWebViewProps = props;
      return R.createElement(View, { testID: props.testID || 'mock-webview' });
    },
  };
});

jest.mock('../../components/Icon', () => {
  const R = require('react');
  const { Text } = require('react-native');
  return function MockIcon(_props: any) {
    return R.createElement(Text, null, '[icon]');
  };
});

import MermaidDiagram from '../../components/MermaidDiagram';

// --- Helpers ----------------------------------------------------------------

const defaultProps = {
  source: 'graph TD\nA --> B',
  accentColor: '#4a90d9',
  textColor: '#333333',
  isDark: false,
};

/** Simulate the WebView posting a "rendered" message with the given height. */
async function postRendered(height: number) {
  expect(lastWebViewProps).toBeTruthy();
  await act(async () => {
    lastWebViewProps.onMessage({
      nativeEvent: { data: JSON.stringify({ type: 'rendered', height }) },
    });
  });
}

/** Extract the numeric height from the WebView style array. */
function getWebViewHeight(): number | undefined {
  const style = lastWebViewProps?.style;
  if (!Array.isArray(style)) return style?.height;
  for (const s of style) {
    if (s && typeof s.height === 'number') return s.height;
  }
  return undefined;
}

// ===========================================================================
// #3722 — XSS: source must be HTML-escaped in the WebView HTML
// ===========================================================================
describe('MermaidDiagram XSS — source HTML-escaping (#3722)', () => {
  beforeEach(() => {
    lastWebViewProps = null;
  });

  it('escapes <img onerror> XSS payload — no raw HTML tag in output', async () => {
    const xssPayload = '<img src=x onerror="alert(1)">';
    await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source: xssPayload })
    );
    const html: string = lastWebViewProps?.source?.html ?? '';
    // The payload must NOT appear as a raw HTML tag
    expect(html).not.toContain('<img src=x onerror');
    // It MUST be HTML-entity-escaped
    expect(html).toContain('&lt;img src=x onerror');
  });

  it('escapes <script> tag injection', async () => {
    const payload = '<script>alert("xss")</script>';
    await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source: payload })
    );
    const html: string = lastWebViewProps?.source?.html ?? '';
    expect(html).not.toContain('<script>alert');
    expect(html).toContain('&lt;script&gt;');
  });

  it('does not double-escape legitimate mermaid syntax', async () => {
    // Mermaid commonly uses < and > in some diagram types (e.g. class diagrams)
    const source = 'classDiagram\nAnimal <|-- Dog';
    await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source })
    );
    const html: string = lastWebViewProps?.source?.html ?? '';
    // The &lt; should be present (escaped), but NOT &amp;lt; (double-escaped)
    expect(html).toContain('&lt;|--');
    expect(html).not.toContain('&amp;lt;');
  });

  it('preserves ampersand by escaping to &amp;', async () => {
    const source = 'graph TD\nA["R&D"] --> B';
    await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source })
    );
    const html: string = lastWebViewProps?.source?.html ?? '';
    expect(html).toContain('R&amp;D');
  });
});

// ===========================================================================
// #3723 — height must update when source/isDark changes
// ===========================================================================
describe('MermaidDiagram height updates on re-render (#3723)', () => {
  beforeEach(() => {
    lastWebViewProps = null;
  });

  it('accepts new height after source change (not stuck at first render)', async () => {
    const { rerender, unmount } = await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source: 'graph TD\nA --> B' })
    );

    // First render: short diagram, height = 100
    await postRendered(100);
    expect(getWebViewHeight()).toBe(100);

    // Re-render with a different (taller) source.
    // Must wrap in act so the useEffect that resets renderedRef fires.
    await act(async () => {
      rerender(
        React.createElement(MermaidDiagram, { ...defaultProps, source: 'graph TD\nA --> B\nB --> C\nC --> D' })
      );
    });

    // Post a new (taller) height — this MUST be accepted, not blocked
    await postRendered(500);
    expect(getWebViewHeight()).toBe(500);
    unmount();
  });

  it('accepts new height after theme toggle', async () => {
    const { rerender, unmount } = await render(
      React.createElement(MermaidDiagram, { ...defaultProps, isDark: false })
    );

    await postRendered(150);
    expect(getWebViewHeight()).toBe(150);

    // Toggle dark mode — wrap in act to flush the useEffect
    await act(async () => {
      rerender(
        React.createElement(MermaidDiagram, { ...defaultProps, isDark: true })
      );
    });

    await postRendered(300);
    expect(getWebViewHeight()).toBe(300);
    unmount();
  });
});
