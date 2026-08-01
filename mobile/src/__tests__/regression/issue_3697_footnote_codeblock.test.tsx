// @ts-nocheck
/**
 * Regression tests for #3697: footnote-definition first pass must ignore
 * fenced code blocks.
 *
 * Previously the first pass scanned ALL lines for `[^id]: text` definitions
 * before code fences were detected, so a `[^id]: ...` line inside a ```
 * fence was consumed and removed from the rendered code block — the block
 * rendered empty and the content showed up as a footnote at the bottom.
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

describe('MarkdownPreview — footnote defs inside code fences (#3697)', () => {
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
    // The code-block content is still rendered (not emptied) — the block is
    // one joined Text node, so match with a regex.
    expect(getByText(/\[\^1\]: this is inside a code block/)).toBeTruthy();
    expect(getByText(/\[\^2\]: also inside/)).toBeTruthy();
    // 'after' (outside the fence) still renders as a paragraph
    expect(getByText('after')).toBeTruthy();
    // The fake definitions must NOT be rendered as footnotes at the bottom
    // (no superscript ref, no definition label, no definition text below)
    expect(queryByText('[1]')).toBeNull();
    expect(queryByText('[2]')).toBeNull();
    expect(queryByText(/this is inside a code block/)).toBeTruthy();
  });

  it('still collects real footnote definitions outside fences', async () => {
    const { getAllByText, getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: 'Paragraph.[^hi]\n\n[^hi]: This is a real footnote definition.',
      })
    );
    // Superscript ref + definition label both render as [hi]
    expect(getAllByText('[hi]').length).toBeGreaterThanOrEqual(1);
    expect(getByText(/real footnote definition/)).toBeTruthy();
    // The raw definition line must not leak as a paragraph
    expect(queryByText(/^\[\^hi\]/)).toBeNull();
  });

  it('mixes code fences and real footnotes on the same note', async () => {
    const { getAllByText, getByText, queryByText } = await render(
      React.createElement(MarkdownPreview, {
        ...defaultProps,
        content: '```\n[^fake]: code block content\n```\n\nReal ref[^real].\n\n[^real]: Real definition.',
      })
    );
    // Code block content preserved
    expect(getByText(/\[\^fake\]: code block content/)).toBeTruthy();
    // Real footnote ref + definition still work
    expect(getAllByText('[real]').length).toBeGreaterThanOrEqual(1);
    expect(getByText(/Real definition/)).toBeTruthy();
    // The in-fence fake line is NOT a footnote
    expect(queryByText('[fake]')).toBeNull();
  });
});
