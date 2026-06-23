/**
 * Regression tests for #1409: latex.ts pure function unit tests.
 *
 * Covers renderLatex, parseLatexSegments, and mapChars.
 */

import { renderLatex, parseLatexSegments } from '../../utils/latex';

// ── renderLatex ──────────────────────────────────────────────

describe('renderLatex', () => {
  // Greek letters
  test('converts Greek letters to Unicode', () => {
    expect(renderLatex('\\alpha')).toBe('α');
    expect(renderLatex('\\beta')).toBe('β');
    expect(renderLatex('\\gamma')).toBe('γ');
    expect(renderLatex('\\Omega')).toBe('Ω');
    expect(renderLatex('\\pi')).toBe('π');
  });

  test('converts multiple Greek letters in one expression', () => {
    expect(renderLatex('\\alpha + \\beta')).toBe('α + β');
  });

  // Fractions
  test('converts fractions to (a/b) format', () => {
    expect(renderLatex('\\frac{a}{b}')).toBe('(a/b)');
    expect(renderLatex('\\frac{1}{2}')).toBe('(1/2)');
    expect(renderLatex('\\frac{x+1}{y-2}')).toBe('(x+1/y-2)');
  });

  // Square roots
  test('converts square roots to √(x) format', () => {
    expect(renderLatex('\\sqrt{x}')).toBe('√(x)');
    expect(renderLatex('\\sqrt{a+b}')).toBe('√(a+b)');
  });

  // Superscripts
  test('converts grouped superscripts', () => {
    expect(renderLatex('x^{2}')).toBe('x²');
    expect(renderLatex('x^{10}')).toBe('x¹⁰');
    expect(renderLatex('a^{n}')).toBe('aⁿ');
  });

  test('converts single-char superscripts', () => {
    expect(renderLatex('x^2')).toBe('x²');
    expect(renderLatex('e^n')).toBe('eⁿ');
  });

  // Subscripts
  test('converts grouped subscripts', () => {
    expect(renderLatex('x_{1}')).toBe('x₁');
    expect(renderLatex('a_{i}')).toBe('aᵢ');
  });

  test('converts single-char subscripts', () => {
    expect(renderLatex('x_1')).toBe('x₁');
    expect(renderLatex('a_i')).toBe('aᵢ');
  });

  // Operators
  test('converts math operators', () => {
    expect(renderLatex('\\cdot')).toBe('·');
    expect(renderLatex('\\times')).toBe('×');
    expect(renderLatex('\\pm')).toBe('±');
    expect(renderLatex('\\leq')).toBe('≤');
    expect(renderLatex('\\geq')).toBe('≥');
    expect(renderLatex('\\neq')).toBe('≠');
    expect(renderLatex('\\approx')).toBe('≈');
    expect(renderLatex('\\infty')).toBe('∞');
  });

  test('converts calculus operators', () => {
    expect(renderLatex('\\int')).toBe('∫');
    expect(renderLatex('\\sum')).toBe('∑');
    expect(renderLatex('\\prod')).toBe('∏');
    expect(renderLatex('\\partial')).toBe('∂');
    expect(renderLatex('\\nabla')).toBe('∇');
  });

  test('converts set operators', () => {
    expect(renderLatex('\\in')).toBe('∈');
    expect(renderLatex('\\notin')).toBe('∉');
    expect(renderLatex('\\subset')).toBe('⊂');
    expect(renderLatex('\\supset')).toBe('⊃');
    expect(renderLatex('\\cup')).toBe('∪');
    expect(renderLatex('\\cap')).toBe('∩');
  });

  test('converts logic operators', () => {
    expect(renderLatex('\\forall')).toBe('∀');
    expect(renderLatex('\\exists')).toBe('∃');
  });

  test('converts arrow operators', () => {
    expect(renderLatex('\\rightarrow')).toBe('→');
    expect(renderLatex('\\leftarrow')).toBe('←');
    expect(renderLatex('\\Rightarrow')).toBe('⇒');
    expect(renderLatex('\\Leftarrow')).toBe('⇐');
  });

  test('converts dots operators', () => {
    expect(renderLatex('\\ldots')).toBe('…');
    expect(renderLatex('\\cdots')).toBe('⋯');
  });

  // Operator word boundary — must not match partial commands
  test('operator matching respects word boundaries', () => {
    // \cdot should not match \cdots
    expect(renderLatex('\\cdots')).toBe('⋯');
    // \le should not match \leftarrow
    expect(renderLatex('\\leftarrow')).toBe('←');
  });

  // Cleanup
  test('removes unknown backslash commands', () => {
    expect(renderLatex('\\unknown{text}')).toBe('text');
  });

  test('removes braces', () => {
    expect(renderLatex('{hello}')).toBe('hello');
  });

  test('collapses extra whitespace', () => {
    expect(renderLatex('a  b  c')).toBe('a b c');
  });

  // Complex expressions
  test('handles complex combined expression', () => {
    const result = renderLatex('\\frac{\\alpha}{\\beta} + \\sqrt{x^2 + y^2}');
    expect(result).toBe('(α/β) + √(x² + y²)');
  });

  // Empty/edge cases
  test('handles empty string', () => {
    expect(renderLatex('')).toBe('');
  });

  test('handles plain text with no LaTeX', () => {
    expect(renderLatex('hello world')).toBe('hello world');
  });
});

// ── parseLatexSegments ───────────────────────────────────────

describe('parseLatexSegments', () => {
  test('parses display math with $$ delimiters', () => {
    const segments = parseLatexSegments('$$x^2$$');
    expect(segments).toHaveLength(1);
    expect(segments[0]).toEqual({
      text: 'x²',
      type: 'latex',
      delimiter: 'display',
    });
  });

  test('parses inline math with $ delimiters', () => {
    const segments = parseLatexSegments('$x^2$');
    expect(segments).toHaveLength(1);
    expect(segments[0]).toEqual({
      text: 'x²',
      type: 'latex',
      delimiter: 'inline',
    });
  });

  test('parses display math with \\[ \\] delimiters', () => {
    const segments = parseLatexSegments('\\[x^2\\]');
    expect(segments).toHaveLength(1);
    expect(segments[0]).toEqual({
      text: 'x²',
      type: 'latex',
      delimiter: 'display',
    });
  });

  test('parses inline math with \\( \\) delimiters', () => {
    const segments = parseLatexSegments('\\(x^2\\)');
    expect(segments).toHaveLength(1);
    expect(segments[0]).toEqual({
      text: 'x²',
      type: 'latex',
      delimiter: 'inline',
    });
  });

  test('parses mixed text and LaTeX', () => {
    const segments = parseLatexSegments('The equation $x^2$ is quadratic');
    expect(segments).toHaveLength(3);
    expect(segments[0]).toEqual({ text: 'The equation ', type: 'text' });
    expect(segments[1]).toEqual({ text: 'x²', type: 'latex', delimiter: 'inline' });
    expect(segments[2]).toEqual({ text: ' is quadratic', type: 'text' });
  });

  test('parses multiple LaTeX segments', () => {
    const segments = parseLatexSegments('$a$ and $b$');
    expect(segments).toHaveLength(3);
    expect(segments[0].type).toBe('latex');
    expect(segments[1]).toEqual({ text: ' and ', type: 'text' });
    expect(segments[2].type).toBe('latex');
  });

  test('returns single text segment for plain text', () => {
    const segments = parseLatexSegments('no math here');
    expect(segments).toHaveLength(1);
    expect(segments[0]).toEqual({ text: 'no math here', type: 'text' });
  });

  test('returns empty array for empty string', () => {
    const segments = parseLatexSegments('');
    expect(segments).toHaveLength(0);
  });

  test('handles Greek letters in parsed segments', () => {
    const segments = parseLatexSegments('$\\alpha + \\beta$');
    expect(segments).toHaveLength(1);
    expect(segments[0].text).toBe('α + β');
  });
});
