/**
 * LaTeX-to-readable-text converter.
 * Converts common LaTeX math expressions to Unicode/text equivalents.
 *
 * Extracted from MarkdownPreview.tsx for unit-testability.
 */
export function renderLatex(tex: string): string {
  let s = tex;
  // Fractions: \frac{a}{b} → (a/b)
  s = s.replace(/\\frac\{([^}]*)\}\{([^}]*)\}/g, '($1/$2)');
  // Square root: \sqrt{x} → √(x)
  s = s.replace(/\\sqrt\{([^}]*)\}/g, '√($1)');
  // Greek letters
  const greek: Record<string, string> = {
    alpha: 'α', beta: 'β', gamma: 'γ', delta: 'δ', epsilon: 'ε',
    zeta: 'ζ', eta: 'η', theta: 'θ', iota: 'ι', kappa: 'κ',
    lambda: 'λ', mu: 'μ', nu: 'ν', xi: 'ξ', pi: 'π',
    rho: 'ρ', sigma: 'σ', tau: 'τ', upsilon: 'υ', phi: 'φ',
    chi: 'χ', psi: 'ψ', omega: 'ω',
    Gamma: 'Γ', Delta: 'Δ', Theta: 'Θ', Lambda: 'Λ', Xi: 'Ξ',
    Pi: 'Π', Sigma: 'Σ', Phi: 'Φ', Psi: 'Ψ', Omega: 'Ω',
    varepsilon: 'ε', varphi: 'ϕ', vartheta: 'ϑ', varrho: 'ϱ',
  };
  for (const [name, char] of Object.entries(greek)) {
    s = s.replace(new RegExp(`\\\\${name}\\b`, 'g'), char);
  }
  // Superscripts and subscripts (single char)
  const sup: Record<string, string> = { '0': '⁰', '1': '¹', '2': '²', '3': '³', '4': '⁴', '5': '⁵', '6': '⁶', '7': '⁷', '8': '⁸', '9': '⁹', '+': '⁺', '-': '⁻', '(': '⁽', ')': '⁾', n: 'ⁿ', i: 'ⁱ' };
  const sub: Record<string, string> = { '0': '₀', '1': '₁', '2': '₂', '3': '₃', '4': '₄', '5': '₅', '6': '₆', '7': '₇', '8': '₈', '9': '₉', '+': '₊', '-': '₋', '(': '₍', ')': '₎', a: 'ₐ', e: 'ₑ', i: 'ᵢ', o: 'ₒ', u: 'ᵤ', x: 'ₓ' };
  s = s.replace(/\^\{([^}]*)\}/g, (_, content) => content.split('').map((c: string) => sup[c] || c).join(''));
  s = s.replace(/_\{([^}]*)\}/g, (_, content) => content.split('').map((c: string) => sub[c] || c).join(''));
  s = s.replace(/\^(\w)/g, (_, c) => sup[c] || `^${c}`);
  s = s.replace(/_(\w)/g, (_, c) => sub[c] || `_${c}`);
  // Common operators — use \b word boundaries to prevent partial matches
  // (e.g. \cdot must not match prefix of \cdots, \le must not match prefix of \leftarrow)
  // Replacements ordered longest-first within groups for safety.
  s = s.replace(/\\cdot\b/g, '·');
  s = s.replace(/\\cdots\b/g, '⋯');
  s = s.replace(/\\ldots\b/g, '…');
  s = s.replace(/\\times\b/g, '×');
  s = s.replace(/\\approx\b/g, '≈');
  s = s.replace(/\\infty\b/g, '∞');
  s = s.replace(/\\partial\b/g, '∂');
  s = s.replace(/\\nabla\b/g, '∇');
  s = s.replace(/\\forall\b/g, '∀');
  s = s.replace(/\\exists\b/g, '∃');
  s = s.replace(/\\rightarrow\b/g, '→');
  s = s.replace(/\\leftarrow\b/g, '←');
  s = s.replace(/\\Rightarrow\b/g, '⇒');
  s = s.replace(/\\Leftarrow\b/g, '⇐');
  s = s.replace(/\\subset\b/g, '⊂');
  s = s.replace(/\\supset\b/g, '⊃');
  s = s.replace(/\\notin\b/g, '∉');
  s = s.replace(/\\prod\b/g, '∏');
  s = s.replace(/\\neq\b/g, '≠');
  s = s.replace(/\\leq\b/g, '≤');
  s = s.replace(/\\geq\b/g, '≥');
  s = s.replace(/\\le\b/g, '≤');
  s = s.replace(/\\ge\b/g, '≥');
  s = s.replace(/\\pm\b/g, '±');
  s = s.replace(/\\mp\b/g, '∓');
  s = s.replace(/\\int\b/g, '∫');
  s = s.replace(/\\sum\b/g, '∑');
  s = s.replace(/\\in\b/g, '∈');
  s = s.replace(/\\cup\b/g, '∪');
  s = s.replace(/\\cap\b/g, '∩');
  // Remove remaining backslash commands (cleanup)
  s = s.replace(/\\[a-zA-Z]+/g, '');
  // Clean up braces
  s = s.replace(/[{}]/g, '');
  // Clean extra spaces
  s = s.replace(/\s{2,}/g, ' ').trim();
  return s;
}
