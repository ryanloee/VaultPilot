/**
 * Unit tests for utils/latex.ts — LaTeX rendering and segment parsing.
 *
 * Extracted from MarkdownPreview.tsx for testability.
 */

import { renderLatex, parseLatexSegments } from '../../utils/latex';

// ── renderLatex ──────────────────────────────────────────

describe('renderLatex', () => {
  // Fractions
  it('converts \\frac{a}{b} to (a/b)', () => {
    expect(renderLatex('\\frac{x+1}{y-2}')).toBe('(x+1/y-2)');
  });

  // Square root
  it('converts \\sqrt{x} to √(x)', () => {
    expect(renderLatex('\\sqrt{42}')).toBe('√(42)');
  });

  it('converts \\sqrt{x^2+y^2}', () => {
    expect(renderLatex('\\sqrt{x^2+y^2}')).toBe('√(x²+y²)');
  });

  // Greek letters
  it('converts Greek letters', () => {
    expect(renderLatex('\\alpha + \\beta')).toBe('α + β');
  });

  it('converts uppercase Greek letters', () => {
    expect(renderLatex('\\Delta \\Sigma')).toBe('Δ Σ');
  });

  it('converts variant Greek letters', () => {
    expect(renderLatex('\\varepsilon \\varphi')).toBe('ε ϕ');
  });

  // Superscripts
  it('converts grouped superscripts', () => {
    expect(renderLatex('x^{10}')).toBe('x¹⁰');
  });

  it('converts single superscript', () => {
    expect(renderLatex('x^2')).toBe('x²');
  });

  // Subscripts
  it('converts single subscript', () => {
    expect(renderLatex('x_1')).toBe('x₁');
  });

  it('converts grouped subscripts', () => {
    // j is not in the sub table → remains 'j'
    expect(renderLatex('a_{ij}')).toBe('aᵢj');
  });

  // Math operators — \cdot must NOT match \cdots
  it('converts \\cdot without matching \\cdots', () => {
    expect(renderLatex('a \\cdot b')).toBe('a · b');
    expect(renderLatex('\\cdots')).toBe('⋯');
  });

  it('converts \\times', () => {
    expect(renderLatex('2 \\times 3')).toBe('2 × 3');
  });

  it('converts \\pm and \\mp', () => {
    expect(renderLatex('\\pm 1')).toBe('± 1');
    expect(renderLatex('\\mp 1')).toBe('∓ 1');
  });

  // \le must NOT match \leftarrow
  it('converts \\le without matching \\leftarrow', () => {
    expect(renderLatex('x \\le y')).toBe('x ≤ y');
    expect(renderLatex('\\leftarrow')).toBe('←');
  });

  it('converts \\leq', () => {
    expect(renderLatex('x \\leq y')).toBe('x ≤ y');
  });

  it('converts \\ge and \\geq', () => {
    expect(renderLatex('x \\ge y')).toBe('x ≥ y');
    expect(renderLatex('x \\geq y')).toBe('x ≥ y');
  });

  it('converts \\ne and \\neq', () => {
    expect(renderLatex('a \\ne b')).toBe('a ≠ b');
    expect(renderLatex('a \\neq b')).toBe('a ≠ b');
  });

  it('converts comparison and calculus operators', () => {
    expect(renderLatex('\\approx')).toBe('≈');
    expect(renderLatex('\\infty')).toBe('∞');
    expect(renderLatex('\\partial')).toBe('∂');
    expect(renderLatex('\\nabla')).toBe('∇');
    expect(renderLatex('\\int')).toBe('∫');
    expect(renderLatex('\\sum')).toBe('∑');
    expect(renderLatex('\\prod')).toBe('∏');
  });

  // \in must NOT match \infty or \int
  it('converts \\in without matching \\infty or \\int', () => {
    expect(renderLatex('x \\in S')).toBe('x ∈ S');
    expect(renderLatex('\\infty')).toBe('∞');
    expect(renderLatex('\\int')).toBe('∫');
  });

  it('converts set operators', () => {
    expect(renderLatex('\\notin')).toBe('∉');
    expect(renderLatex('\\subset')).toBe('⊂');
    expect(renderLatex('\\supset')).toBe('⊃');
    expect(renderLatex('\\cup')).toBe('∪');
    expect(renderLatex('\\cap')).toBe('∩');
  });

  it('converts logic operators', () => {
    expect(renderLatex('\\forall')).toBe('∀');
    expect(renderLatex('\\exists')).toBe('∃');
  });

  it('converts arrow operators', () => {
    expect(renderLatex('\\rightarrow')).toBe('→');
    expect(renderLatex('\\leftarrow')).toBe('←');
    expect(renderLatex('\\Rightarrow')).toBe('⇒');
    expect(renderLatex('\\Leftarrow')).toBe('⇐');
  });

  it('converts dots', () => {
    expect(renderLatex('\\ldots')).toBe('…');
    expect(renderLatex('\\cdots')).toBe('⋯');
  });

  // Cleanup
  it('removes remaining backslash commands', () => {
    expect(renderLatex('\\text{hello}')).toBe('hello');
  });

  it('removes braces', () => {
    expect(renderLatex('{x}')).toBe('x');
  });

  it('collapses extra spaces', () => {
    expect(renderLatex('a  b   c')).toBe('a b c');
  });

  // Complex expressions
  it('converts simple fraction expression', () => {
    const result = renderLatex('\\frac{1}{2}');
    expect(result).toBe('(1/2)');
  });

  it('converts Euler identity', () => {
    // e^{i\pi} → Greek: e^{iπ} → superscript: eⁱπ
    const result = renderLatex('e^{i\\pi} + 1 = 0');
    expect(result).toBe('eⁱπ + 1 = 0');
  });

  // Empty input
  it('handles empty string', () => {
    expect(renderLatex('')).toBe('');
  });

  // Regression: operators with \b boundaries
  it('does not mangle \\leq prefix of \\leftarrow', () => {
    // This was a bug: \\leq? without \\b would match \\le in \\leftarrow
    expect(renderLatex('\\leftarrow')).toBe('←');
    expect(renderLatex('\\rightarrow')).toBe('→');
    expect(renderLatex('\\Leftarrow')).toBe('⇐');
    expect(renderLatex('\\Rightarrow')).toBe('⇒');
  });

  it('does not mangle \\cdot prefix of \\cdots', () => {
    expect(renderLatex('\\cdot')).toBe('·');
    expect(renderLatex('\\cdots')).toBe('⋯');
  });
});

// ── parseLatexSegments ───────────────────────────────────

describe('parseLatexSegments', () => {
  it('returns single text segment for plain text', () => {
    const segs = parseLatexSegments('hello world');
    expect(segs).toHaveLength(1);
    expect(segs[0]).toEqual({ text: 'hello world', type: 'text' });
  });

  it('parses inline math with $ delimiters', () => {
    const segs = parseLatexSegments('The value is $x^2$ here');
    expect(segs).toHaveLength(3);
    expect(segs[0]).toEqual({ text: 'The value is ', type: 'text' });
    expect(segs[1]).toEqual({ text: 'x²', type: 'latex', delimiter: 'inline' });
    expect(segs[2]).toEqual({ text: ' here', type: 'text' });
  });

  it('parses display math with $$ delimiters', () => {
    const segs = parseLatexSegments('See $$\\frac{a}{b}$$ below');
    expect(segs).toHaveLength(3);
    expect(segs[1]).toEqual({ text: '(a/b)', type: 'latex', delimiter: 'display' });
  });

  it('parses inline math with \\( \\) delimiters', () => {
    const segs = parseLatexSegments('Value: \\(x + 1\\) end');
    expect(segs).toHaveLength(3);
    expect(segs[1]).toEqual({ text: 'x + 1', type: 'latex', delimiter: 'inline' });
  });

  it('parses display math with \\[ \\] delimiters', () => {
    const segs = parseLatexSegments('See \\[\\sum_{i=1}^n i\\] below');
    expect(segs).toHaveLength(3);
    expect(segs[1].type).toBe('latex');
    expect(segs[1].delimiter).toBe('display');
    expect(segs[1].text).toContain('∑');
  });

  it('parses multiple inline expressions', () => {
    const segs = parseLatexSegments('$a$ and $b$');
    expect(segs).toHaveLength(3);
    expect(segs[0]).toEqual({ text: 'a', type: 'latex', delimiter: 'inline' });
    expect(segs[1]).toEqual({ text: ' and ', type: 'text' });
    expect(segs[2]).toEqual({ text: 'b', type: 'latex', delimiter: 'inline' });
  });

  it('handles text with no math', () => {
    const segs = parseLatexSegments('no math here');
    expect(segs).toHaveLength(1);
    expect(segs[0].type).toBe('text');
  });

  it('handles empty string', () => {
    const segs = parseLatexSegments('');
    expect(segs).toHaveLength(0);
  });

  it('handles math at start of text', () => {
    const segs = parseLatexSegments('$x$ is a variable');
    expect(segs).toHaveLength(2);
    expect(segs[0].type).toBe('latex');
    expect(segs[1]).toEqual({ text: ' is a variable', type: 'text' });
  });

  it('handles math at end of text', () => {
    const segs = parseLatexSegments('result is $x$');
    expect(segs).toHaveLength(2);
    expect(segs[0]).toEqual({ text: 'result is ', type: 'text' });
    expect(segs[1].type).toBe('latex');
  });
});
