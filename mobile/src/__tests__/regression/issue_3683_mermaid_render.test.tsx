// @ts-nocheck
/**
 * Regression test for #3683: Mermaid diagram SVG rendering.
 *
 * Previously, mermaid code blocks only showed raw text in a card (#2805).
 * Now they render as actual SVG charts via MermaidDiagram + WebView.
 *
 * Tests that:
 * - MermaidDiagram renders with header and WebView
 * - Works in dark and light mode
 * - Handles empty and complex sources
 */

import React from 'react';
import { render } from '@testing-library/react-native';

// Mock react-native-webview
jest.mock('react-native-webview', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    WebView: (props: any) => React.createElement(View, { testID: props.testID || 'mock-webview' }),
  };
});

// Mock Icon component
jest.mock('../../components/Icon', () => {
  const React = require('react');
  const { Text } = require('react-native');
  return function MockIcon(_props: any) {
    return React.createElement(Text, null, '[icon]');
  };
});

import MermaidDiagram from '../../components/MermaidDiagram';

describe('MermaidDiagram component (#3683)', () => {
  const defaultProps = {
    source: 'graph TD\nA --> B\nB --> C',
    accentColor: '#4a90d9',
    textColor: '#333333',
    isDark: false,
  };

  it('renders a card with header label and WebView', async () => {
    const { getByTestId, getByText } = await render(
      React.createElement(MermaidDiagram, defaultProps)
    );
    expect(getByTestId('mermaid-diagram-card')).toBeTruthy();
    expect(getByText('Mermaid Diagram')).toBeTruthy();
    expect(getByTestId('mermaid-webview')).toBeTruthy();
  });

  it('renders correctly in dark mode', async () => {
    const { getByTestId } = await render(
      React.createElement(MermaidDiagram, { ...defaultProps, isDark: true })
    );
    expect(getByTestId('mermaid-diagram-card')).toBeTruthy();
  });

  it('handles empty source without crashing', async () => {
    const { getByTestId } = await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source: '' })
    );
    expect(getByTestId('mermaid-diagram-card')).toBeTruthy();
  });

  it('renders with complex flowchart source', async () => {
    const complexSource = [
      'graph TD',
      'A[Start] --> B{Decision}',
      'B -->|Yes| C[Action 1]',
      'B -->|No| D[Action 2]',
      'C --> E[End]',
      'D --> E',
    ].join('\n');
    const { getByTestId } = await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source: complexSource })
    );
    expect(getByTestId('mermaid-webview')).toBeTruthy();
  });

  it('renders with sequence diagram source', async () => {
    const seqSource = [
      'sequenceDiagram',
      'participant Alice',
      'participant Bob',
      'Alice->>Bob: Hello Bob',
      'Bob-->>Alice: Hello Alice',
    ].join('\n');
    const { getByTestId } = await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source: seqSource })
    );
    expect(getByTestId('mermaid-diagram-card')).toBeTruthy();
  });

  it('renders with Gantt chart source', async () => {
    const ganttSource = [
      'gantt',
      'title A Gantt Diagram',
      'dateFormat YYYY-MM-DD',
      'section Section',
      'A task :a1, 2024-01-01, 30d',
      'Another task :after a1, 20d',
    ].join('\n');
    const { getByTestId } = await render(
      React.createElement(MermaidDiagram, { ...defaultProps, source: ganttSource })
    );
    expect(getByTestId('mermaid-webview')).toBeTruthy();
  });
});
