/**
 * Regression test for #3945 — Lightbox violates React Rules of Hooks.
 *
 * Bug: `Lightbox.tsx` had an early `return null` for empty images BEFORE any
 * hooks were called. When `images` transitioned from non-empty → empty across
 * renders, the number of hooks called changed, causing React to throw
 * "Rendered fewer/more hooks than expected".
 *
 * Fix: all hooks are now called unconditionally; the empty-images guard was
 * moved after the last hook.
 *
 * This test renders the component first with a non-empty images array, then
 * re-renders with an empty array, and asserts no React hooks-order error is
 * thrown — mirroring the exact crash scenario from the bug report.
 *
 * Uses `react-test-renderer` (synchronous) so a hooks-order violation throws
 * immediately inside the test rather than surfacing as an unhandled rejection.
 */
import React from 'react';
import { create, act } from 'react-test-renderer';
import Lightbox from '../../components/Lightbox';
import type { MarkdownImage } from '../../utils/imageMarkdown';

jest.mock('../../components/Icon', () => {
  const React = require('react');
  return {
    __esModule: true,
    default: (props: any) => React.createElement('Icon', props),
  };
});

const sampleImage: MarkdownImage = {
  uri: 'file:///test.png',
  alt: 'test image',
};

describe('#3945 — Lightbox Rules of Hooks with empty images', () => {
  it('does not throw when images transitions from non-empty to empty', () => {
    // First render with a non-empty images array — all hooks are registered.
    let renderer: any;
    act(() => {
      renderer = create(
        <Lightbox
          visible
          images={[sampleImage]}
          index={0}
          onClose={jest.fn()}
        />,
      );
    });

    // Second render with an EMPTY images array.
    // Before the fix this raised:
    //   "Rendered fewer hooks than expected."
    // because the early `return null` skipped all the hook calls.
    expect(() => {
      act(() => {
        renderer.update(
          <Lightbox
            visible
            images={[]}
            index={0}
            onClose={jest.fn()}
          />,
        );
      });
    }).not.toThrow();
  });

  it('renders null for empty images without crashing', () => {
    let renderer: any;
    act(() => {
      renderer = create(
        <Lightbox visible images={[]} index={0} onClose={jest.fn()} />,
      );
    });
    // When images is empty, the component returns null.
    expect(renderer.toJSON()).toBeNull();
  });

  it('does not throw when images transitions from empty to non-empty', () => {
    let renderer: any;
    // First render with an empty images array.
    act(() => {
      renderer = create(
        <Lightbox visible images={[]} index={0} onClose={jest.fn()} />,
      );
    });

    // Second render with a non-empty images array.
    // Before the fix, going from 0 hooks → N hooks also threw.
    expect(() => {
      act(() => {
        renderer.update(
          <Lightbox
            visible
            images={[sampleImage]}
            index={0}
            onClose={jest.fn()}
          />,
        );
      });
    }).not.toThrow();
  });
});
