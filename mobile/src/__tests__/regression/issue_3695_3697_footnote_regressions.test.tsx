// @ts-nocheck
/**
 * Regression tests for #3695 and #3697 — MarkdownPreview footnote rendering.
 *
 * #3695: commit db8a9112 moved the footnote-ref match ([^id]) BEFORE the
 * bold/italic checks, so `**bold [^1]**` rendered as literal `**bold ` +
 * superscript + stray `*` `*` (bold destroyed). It also moved fnMatch before
 * linkMatch, so the valid GFM link `[^1](url)` was eaten by the fn matcher.
 *
 * Fix: fn refs are matched AFTER bold/italic/link, and bold/italic span
 * content is rendered through a nested pass (renderSpanWithFnRefs) that
 * converts `[^id]` to superscripts without destroying the delimiters.
 *
 * #3697: the footnote-definition first pass ignored fenced code blocks, so a
 * `[^id]: text` line inside a ``` block was consumed as a definition and the
 * code block rendered empty. Fix: first pass is now fence-aware.
 */

import React from 'react';
import { render } from '@testing-library/react-native';
import MarkdownPreview from '../../components/MarkdownPreview';

// Mock Icon to avoid SVG dep
jest.mock('../../components/Icon', () => {
  const { Text } = require('react-native');
  return function MockIcon(_props: any) {
    return React.createElement(Text, null, '[icon]');
  };
});

// Mock Lightbox to avoid native module dependencies
jest.mock('../../components/Lightbox', () => {
  const { View } = require('react-native');
  return function MockLightbox(_props: any) {
    return React.createElement(View, { testID: 'mock-lightbox' });
  };
});

describe('MarkdownPreview — footnote refs vs bold/italic (#3695)', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
    content: '',
  };

  it('renders **bold [^1]** as bold text + superscript, no stray asterisks', async () => {
    const { getAllByText, getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '**bold [^1]**\n\n[^1]: def text',
      })
    );
    // Superscript ref still rendered inside the bold span (plus the
    // definition footer also shows [1] — both are expected).
    expect(getAllByText('[1]').length).toBeGreaterThan(0);
    // Bold text content preserved
    expect(getByText(/bold/)).toBeTruthy();
    // No literal asterisks survive — the delimiters are not leaked
    expect(queryByText(/\*/)).toBeNull();
  });

  it('renders *italic [^1]* as italic text + superscript, no stray asterisks', async () => {
    const { getAllByText, getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '*italic [^1]*\n\n[^1]: def text',
      })
    );
    expect(getAllByText('[1]').length).toBeGreaterThan(0);
    expect(getByText(/italic/)).toBeTruthy();
    expect(queryByText(/\*/)).toBeNull();
  });

  it('renders [^1](url) as a GFM link with text ^1, not a footnote ref', async () => {
    const { getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '[^1](https://example.com)',
      })
    );
    // Link text `^1` renders underlined as a link
    expect(getByText('^1')).toBeTruthy();
    // It must NOT be converted to a footnote superscript `[1]`
    expect(queryByText('[1]')).toBeNull();
  });

  it('still renders plain footnote refs outside formatting', async () => {
    const { getByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Some text with a footnote[^1] here.',
      })
    );
    expect(getByText('[1]')).toBeTruthy();
  });
});

describe('MarkdownPreview — footnote-definition first pass is fence-aware (#3697)', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
    content: '',
  };

  it('keeps [^id]: lines inside a fenced code block as code content', async () => {
    const { getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '```\n[^1]: this is inside a code block\n[^2]: also inside\n```\nafter',
      })
    );
    // The code block must contain both lines (not render empty)
    expect(getByText(/this is inside a code block/)).toBeTruthy();
    expect(getByText(/also inside/)).toBeTruthy();
    // No footnote definitions may be extracted from inside the fence
    expect(queryByText('[1]')).toBeNull();
    expect(queryByText('[2]')).toBeNull();
  });

  it('still extracts real footnote definitions outside fences', async () => {
    const { getAllByText, getByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'text[^3]\n\n```\n[^1]: code line\n```\n\n[^3]: real definition',
      })
    );
    // The real definition outside the fence is rendered at the bottom
    expect(getByText(/real definition/)).toBeTruthy();
    // The ref in the paragraph becomes a superscript (footer [3] too)
    expect(getAllByText('[3]').length).toBeGreaterThan(0);
    // The code block keeps its line
    expect(getByText(/code line/)).toBeTruthy();
  });
});
