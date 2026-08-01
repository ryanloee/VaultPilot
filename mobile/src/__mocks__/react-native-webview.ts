/**
 * Jest mock for react-native-webview (#3683).
 *
 * The real module ships ESM (`import WebView from './lib/WebView'`) which
 * Jest's ts-jest transform cannot parse from node_modules. Every other
 * native dependency in this project is stubbed via moduleNameMapper in
 * jest.config.js; this follows the same pattern so that components which
 * render MermaidDiagram (and therefore WebView) load under test.
 *
 * Only the surface area actually used by MermaidDiagram.tsx is stubbed:
 *   - default export WebView  (rendered as <WebView .../>)
 *   - WebViewMessageEvent     (type-only, erased at compile time)
 */
import React from 'react';

export type WebViewMessageEvent = {
  nativeEvent: { data: string };
};

const WebView = (props: any) =>
  React.createElement('WebView', props, props.children);

export default WebView;
