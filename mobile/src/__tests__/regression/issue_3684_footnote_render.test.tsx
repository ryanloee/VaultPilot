// @ts-nocheck
/**
 * Regression tests for #3684: Mobile Markdown footnote rendering.
 *
 * GFM-style footnotes: [^id] references in text + [^id]: definition at end.
 * The first pass collects definitions and hides them from normal rendering;
 * inline refs are rendered as superscript; definitions rendered at bottom.
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

describe('MarkdownPreview — footnote rendering (#3684)', () => {
  const defaultProps = {
    textColor: '#000',
    accentColor: '#007bff',
    isDark: false,
    content: '',
  };

  it('renders [^id] inline refs as superscript text', async () => {
    const { getByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Some text with a footnote[^1] here.',
      })
    );
    expect(getByText('[1]')).toBeTruthy();
  });

  it('renders footnote definitions at bottom', async () => {
    const { getByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Paragraph.[^hi]\n\n[^hi]: This is a footnote definition.',
      })
    );
    expect(getByText(/footnote definition/)).toBeTruthy();
  });

  it('renders [^id] ref without matching definition', async () => {
    const { getByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Some [^orphan] ref with no definition.',
      })
    );
    // The ref still renders inline even without a definition
    expect(getByText('[orphan]')).toBeTruthy();
  });

  it('skips rendering definition lines as normal paragraphs', async () => {
    const { queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Text before.\n\n[^fn]: This definition should not appear as a regular paragraph.\n\nText after.',
      })
    );
    // The raw definition line prefix should not appear in output
    expect(queryByText(/^\[\^fn\]/)).toBeNull();
  });
});
