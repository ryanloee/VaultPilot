/**
 * LaTeX-to-readable-text converter.
 * Extracted from MarkdownPreview.tsx for testability.
 *
 * Converts common LaTeX math expressions to Unicode/text equivalents.
 */

/** Greek letters mapping */
const GREEK: Record<string, string> = {
  alpha: 'α', beta: 'β', gamma: 'γ', delta: 'δ', epsilon: 'ε',
  zeta: 'ζ', eta: 'η', theta: 'θ', iota: 'ι', kappa: 'κ',
  lambda: 'λ', mu: 'μ', nu: 'ν', xi: 'ξ', pi: 'π',
  rho: 'ρ', sigma: 'σ', tau: 'τ', upsilon: 'υ', phi: 'φ',
  chi: 'χ', psi: 'ψ', omega: 'ω',
  Gamma: 'Γ', Delta: 'Δ', Theta: 'Θ', Lambda: 'Λ', Xi: 'Ξ',
  Pi: 'Π', Sigma: 'Σ', Phi: 'Φ', Psi: 'Ψ', Omega: 'Ω',
  varepsilon: 'ε', varphi: 'ϕ', vartheta: 'ϑ', varrho: 'ϱ',
};

/** Superscript character mapping */
const SUP: Record<string, string> = {
  '0': '⁰', '1': '¹', '2': '²', '3': '³', '4': '⁴',
  '5': '⁵', '6': '⁶', '7': '⁷', '8': '⁸', '9': '⁹',
  '+': '⁺', '-': '⁻', '(': '⁽', ')': '⁾', n: 'ⁿ', i: 'ⁱ',
};

/** Subscript character mapping */
const SUB: Record<string, string> = {
  '0': '₀', '1': '₁', '2': '₂', '3': '₃', '4': '₄',
  '5': '₅', '6': '₆', '7': '₇', '8': '₈', '9': '₉',
  '+': '₊', '-': '₋', '(': '₍', ')': '₎',
  a: 'ₐ', e: 'ₑ', i: 'ᵢ', o: 'ₒ', u: 'ᵤ', x: 'ₓ',
};

function mapChars(text: string, table: Record<string, string>): string {
  return text.split('').map(c => table[c] || c).join('');
}

/**
 * Convert LaTeX math expression to Unicode text.
 * Handles fractions, square roots, Greek letters, superscripts/subscripts,
 * and common math operators.
 */
export function renderLatex(tex: string): string {
  let s = tex;
  // Fractions: \frac{a}{b} → (a/b)
  s = s.replace(/\\frac\{([^}]*)\}\{([^}]*)\}/g, '($1/$2)');
  // Square root: \sqrt{x} → √(x)
  s = s.replace(/\\sqrt\{([^}]*)\}/g, '√($1)');
  // Greek letters
  for (const [name, char] of Object.entries(GREEK)) {
    s = s.replace(new RegExp(`\\\\${name}\\b`, 'g'), char);
  }
  // Superscripts and subscripts (grouped: ^{...}, _{...})
  s = s.replace(/\^{([^}]*)}/g, (_, content) => mapChars(content, SUP));
  s = s.replace(/_{([^}]*)}/g, (_, content) => mapChars(content, SUB));
  // Superscripts and subscripts (single char: ^x, _x)
  s = s.replace(/\^(\w)/g, (_, c) => SUP[c] || `^${c}`);
  s = s.replace(/_(\w)/g, (_, c) => SUB[c] || `_${c}`);
  // Common operators — use \b word boundary to avoid partial matches
  // (e.g. \cdot must not match \cdots, \le must not match \leftarrow)
  s = s.replace(/\\cdot\b/g, '·');
  s = s.replace(/\\times\b/g, '×');
  s = s.replace(/\\pm\b/g, '±');
  s = s.replace(/\\mp\b/g, '∓');
  s = s.replace(/\\leq?\b/g, '≤');
  s = s.replace(/\\geq?\b/g, '≥');
  s = s.replace(/\\neq?\b/g, '≠');
  s = s.replace(/\\approx\b/g, '≈');
  s = s.replace(/\\infty\b/g, '∞');
  s = s.replace(/\\partial\b/g, '∂');
  s = s.replace(/\\nabla\b/g, '∇');
  s = s.replace(/\\int\b/g, '∫');
  s = s.replace(/\\sum\b/g, '∑');
  s = s.replace(/\\prod\b/g, '∏');
  s = s.replace(/\\in\b/g, '∈');
  s = s.replace(/\\notin\b/g, '∉');
  s = s.replace(/\\subset\b/g, '⊂');
  s = s.replace(/\\supset\b/g, '⊃');
  s = s.replace(/\\cup\b/g, '∪');
  s = s.replace(/\\cap\b/g, '∩');
  s = s.replace(/\\forall\b/g, '∀');
  s = s.replace(/\\exists\b/g, '∃');
  s = s.replace(/\\rightarrow\b/g, '→');
  s = s.replace(/\\leftarrow\b/g, '←');
  s = s.replace(/\\Rightarrow\b/g, '⇒');
  s = s.replace(/\\Leftarrow\b/g, '⇐');
  s = s.replace(/\\ldots\b/g, '…');
  s = s.replace(/\\cdots\b/g, '⋯');
  // Remove remaining backslash commands (cleanup)
  s = s.replace(/\\[a-zA-Z]+/g, '');
  // Clean up braces
  s = s.replace(/[{}]/g, '');
  // Clean extra spaces
  s = s.replace(/\s{2,}/g, ' ').trim();
  return s;
}

/** Delimiter type for a LaTeX segment */
export type LatexDelimiter = 'display' | 'inline';

/** A parsed LaTeX segment */
export interface LatexSegment {
  text: string;
  type: 'text' | 'latex';
  delimiter?: LatexDelimiter;
}

/**
 * Parse text into alternating text and LaTeX segments.
 * Display math: $$...$$ or \[...\]
 * Inline math: $...$ or \(...\)
 */
export function parseLatexSegments(text: string): LatexSegment[] {
  const segments: LatexSegment[] = [];
  const pattern = /(\$\$[\s\S]*?\$\$|\\\[[\s\S]*?\\\]|\\\$[^$\n]+?\\\$|\\\([\s\S]*?\\\))/g;
  let lastIndex = 0;
  let match;

  while ((match = pattern.exec(text)) !== null) {
    if (match.index > lastIndex) {
      segments.push({ text: text.slice(lastIndex, match.index), type: 'text' });
    }

    let tex = match[0];
    const isDisplay = tex.startsWith('$$') || tex.startsWith('\\[');
    // Strip delimiters
    if (tex.startsWith('$$')) tex = tex.slice(2, -2);
    else if (tex.startsWith('\\[')) tex = tex.slice(2, -2);
    else if (tex.startsWith('\\(')) tex = tex.slice(2, -2);
    else tex = tex.slice(1, -1);

    segments.push({
      text: renderLatex(tex),
      type: 'latex',
      delimiter: isDisplay ? 'display' : 'inline',
    });
    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < text.length) {
    segments.push({ text: text.slice(lastIndex), type: 'text' });
  }

  return segments;
}
