// @ts-nocheck
/**
 * Regression test for #3724 — duplicate React keys in renderInline.
 *
 * In MarkdownPreview.tsx, renderInline()'s plain-text fallback branch never
 * incremented the `key` counter. When content had multiple text segments
 * separated by `[` (e.g. "See [Section 3] and [Appendix A]"), every segment
 * received the same key `t0`, producing duplicate React keys among siblings.
 *
 * Fix: increment `key` after each renderWithNoteRefs call in all three
 * plain-text branches (nextSpecial > 0, nextSpecial === -1, single-char).
 *
 * Test: spy on console.error to detect React's "Encountered two children with
 * the same key" warning.
 */

import React from 'react';
import { render } from '@testing-library/react-native';
import MarkdownPreview from '../../components/MarkdownPreview';

// Mock Icon
jest.mock('../../components/Icon', () => {
  const R = require('react');
  const { Text } = require('react-native');
  return function MockIcon(_props: any) {
    return R.createElement(Text, null, '[icon]');
  };
});

// Mock Lightbox
jest.mock('../../components/Lightbox', () => {
  const R = require('react');
  const { View } = require('react-native');
  return function MockLightbox(_props: any) {
    return R.createElement(View, { testID: 'mock-lightbox' });
  };
});

// Mock react-native-webview (pulled in via MermaidDiagram)
jest.mock('react-native-webview', () => {
  const R = require('react');
  const { View } = require('react-native');
  return {
    WebView: (props: any) => R.createElement(View, { testID: props.testID || 'mock-webview' }),
  };
});

describe('MarkdownPreview — no duplicate React keys (#3724)', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
    content: '',
  };

  let consoleErrorSpy: jest.SpyInstance;

  beforeEach(() => {
    consoleErrorSpy = jest.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    consoleErrorSpy.mockRestore();
  });

  /** Returns true if any console.error call mentions duplicate keys. */
  function hasDuplicateKeyWarning(): boolean {
    return consoleErrorSpy.mock.calls.some(
      (call: any[]) =>
        typeof call[0] === 'string' &&
        (call[0].includes('same key') ||
          call[0].includes('Encountered two children'))
    );
  }

  it('does not warn about duplicate keys with multiple brackets in text', async () => {
    // Content with multiple `[` chars that are NOT valid markdown links.
    // Before the fix, each plain-text segment got key `t0`.
    await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'See [Section 3] and [Appendix A] for details.',
      })
    );
    expect(hasDuplicateKeyWarning()).toBe(false);
  });

  it('does not warn about duplicate keys with mixed brackets and special chars', async () => {
    await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Mix [bracket] with `code` and *italic* and [another] end.',
      })
    );
    expect(hasDuplicateKeyWarning()).toBe(false);
  });

  it('does not warn about duplicate keys with many consecutive brackets', async () => {
    await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'a[b]c[d]e[f]g[h]i[j]k',
      })
    );
    expect(hasDuplicateKeyWarning()).toBe(false);
  });

  it('does not warn about duplicate keys with standalone opening bracket', async () => {
    // A lone `[` not followed by `]` is treated as plain text
    await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Text with [ lone bracket in middle',
      })
    );
    expect(hasDuplicateKeyWarning()).toBe(false);
  });
});
