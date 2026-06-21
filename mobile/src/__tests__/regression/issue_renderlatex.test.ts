import { renderLatex } from '../../utils/latex';

describe('renderLatex', () => {
  // ── Fractions ──
  it('converts \\frac{a}{b} to (a/b)', () => {
    expect(renderLatex('\\frac{1}{2}')).toBe('(1/2)');
  });

  it('converts nested fraction', () => {
    expect(renderLatex('\\frac{x+1}{y-2}')).toBe('(x+1/y-2)');
  });

  // ── Square root ──
  it('converts \\sqrt{x} to √(x)', () => {
    expect(renderLatex('\\sqrt{4}')).toBe('√(4)');
  });

  // ── Greek letters ──
  it('converts lowercase Greek letters', () => {
    expect(renderLatex('\\alpha + \\beta')).toBe('α + β');
  });

  it('converts uppercase Greek letters', () => {
    expect(renderLatex('\\Gamma \\Delta')).toBe('Γ Δ');
  });

  it('converts var- Greek letters', () => {
    expect(renderLatex('\\varepsilon')).toBe('ε');
    expect(renderLatex('\\varphi')).toBe('ϕ');
  });

  // ── Superscripts and subscripts ──
  it('converts single-char superscript', () => {
    expect(renderLatex('x^2')).toBe('x²');
  });

  it('converts braced superscript', () => {
    expect(renderLatex('x^{10}')).toBe('x¹⁰');
  });

  it('converts single-char subscript', () => {
    expect(renderLatex('a_0')).toBe('a₀');
  });

  it('converts braced subscript', () => {
    // n is not in the sub map, so it passes through as-is
    expect(renderLatex('a_{n+1}')).toBe('an₊₁');
  });

  // ── Operators ──
  it('converts common operators', () => {
    expect(renderLatex('\\cdot')).toBe('·');
    expect(renderLatex('\\times')).toBe('×');
    expect(renderLatex('\\pm')).toBe('±');
    expect(renderLatex('\\infty')).toBe('∞');
  });

  it('converts comparison operators', () => {
    expect(renderLatex('\\leq')).toBe('≤');
    expect(renderLatex('\\geq')).toBe('≥');
    expect(renderLatex('\\neq')).toBe('≠');
    expect(renderLatex('\\approx')).toBe('≈');
  });

  // ── Set/logic operators ──
  it('converts set operators', () => {
    expect(renderLatex('\\in')).toBe('∈');
    expect(renderLatex('\\notin')).toBe('∉');
    expect(renderLatex('\\subset')).toBe('⊂');
    expect(renderLatex('\\cup')).toBe('∪');
    expect(renderLatex('\\cap')).toBe('∩');
  });

  // ── Arrows ──
  it('converts arrows', () => {
    expect(renderLatex('\\rightarrow')).toBe('→');
    expect(renderLatex('\\leftarrow')).toBe('←');
    expect(renderLatex('\\Rightarrow')).toBe('⇒');
    expect(renderLatex('\\Leftarrow')).toBe('⇐');
  });

  // ── Calculus ──
  it('converts calculus symbols', () => {
    expect(renderLatex('\\int')).toBe('∫');
    expect(renderLatex('\\sum')).toBe('∑');
    expect(renderLatex('\\prod')).toBe('∏');
    expect(renderLatex('\\partial')).toBe('∂');
    expect(renderLatex('\\nabla')).toBe('∇');
  });

  // ── Dots ──
  it('converts dots', () => {
    expect(renderLatex('\\ldots')).toBe('…');
    expect(renderLatex('\\cdots')).toBe('⋯');
  });

  // ── Cleanup ──
  it('removes unknown backslash commands', () => {
    expect(renderLatex('\\text{hello}')).toBe('hello');
  });

  it('removes braces', () => {
    expect(renderLatex('{x}')).toBe('x');
  });

  it('collapses extra spaces', () => {
    expect(renderLatex('a   b')).toBe('a b');
  });

  it('handles empty string', () => {
    expect(renderLatex('')).toBe('');
  });

  it('handles plain text passthrough', () => {
    expect(renderLatex('hello world')).toBe('hello world');
  });

  // ── Combined expressions ──
  it('converts a realistic LaTeX expression', () => {
    // E = mc^2
    expect(renderLatex('E = mc^2')).toBe('E = mc²');
  });

  it('converts integral expression', () => {
    expect(renderLatex('\\int_0^1 x^2 dx')).toBe('∫₀¹ x² dx');
  });
});
