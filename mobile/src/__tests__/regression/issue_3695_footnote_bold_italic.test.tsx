// @ts-nocheck
/**
 * Regression tests for #3695: footnote refs inside bold/italic spans.
 *
 * db8a9112 (fix #3690) moved the footnote-ref match before bold/italic,
 * which broke `**bold [^1]**` — the `**` delimiters rendered as literal
 * stray asterisks. The fix restores the correct precedence:
 *   bold/italic > [link](url) > [^id] footnote ref
 * with a nested footnote-ref pass for formatted span content.
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

describe('MarkdownPreview — footnote refs in bold/italic (#3695)', () => {
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
    // Footnote ref still renders as superscript (also appears as the
    // definition label at the bottom — hence getAllByText)
    expect(getAllByText('[1]').length).toBeGreaterThanOrEqual(1);
    // Bold span content kept its text (no literal ** around it)
    expect(getByText('bold ')).toBeTruthy();
    // No stray asterisk characters anywhere
    expect(queryByText('*')).toBeNull();
    expect(queryByText('**')).toBeNull();
  });

  it('renders *italic [^1]* as italic text + superscript, no stray asterisks', async () => {
    const { getAllByText, getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '*italic [^1]*\n\n[^1]: def text',
      })
    );
    expect(getAllByText('[1]').length).toBeGreaterThanOrEqual(1);
    expect(getByText('italic ')).toBeTruthy();
    expect(queryByText('*')).toBeNull();
  });

  it('renders [^1](https://example.com) as a link with text ^1, not superscript + literal URL', async () => {
    const { getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'See [^1](https://example.com) here.',
      })
    );
    // Link text is `^1` (valid GFM link), rendered as underlined link
    expect(getByText('^1')).toBeTruthy();
    // NOT hijacked into a footnote superscript
    expect(queryByText('[1]')).toBeNull();
  });

  it('keeps plain footnote refs working outside formatted spans', async () => {
    const { getAllByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Some text with a footnote[^1] here.\n\n[^1]: def text',
      })
    );
    expect(getAllByText('[1]').length).toBeGreaterThanOrEqual(1);
  });

  it('handles footnote ref inside bold followed by a link on same line', async () => {
    const { getAllByText, getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '**bold [^1]** and [link](https://example.com)\n\n[^1]: def text',
      })
    );
    expect(getAllByText('[1]').length).toBeGreaterThanOrEqual(1);
    expect(getByText('bold ')).toBeTruthy();
    expect(getByText('link')).toBeTruthy();
    expect(queryByText('*')).toBeNull();
  });
});
