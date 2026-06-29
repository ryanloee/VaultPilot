import React from 'react';
import { Text, View, StyleSheet, ScrollView, TouchableOpacity } from 'react-native';
import { renderLatex, parseLatexSegments } from '../utils/latex';
import Icon from './Icon';
import { findNoteReferences, splitLineByNoteRefs } from '../utils/noteRefs';

interface Props {
  content: string;
  textColor: string;
  accentColor: string;
  isDark: boolean;
  /** Called when a [[wikilink]] or auto-detected note ref is tapped — receives the note title as argument. */
  onNoteLinkPress?: (title: string) => void;
  /** Map of note title (lowercase) → noteId for auto-detection of note references (#2035). */
  noteTitleMap?: Map<string, string>;
}

/**
 * Process text to find and render LaTeX expressions.
 * Display math: $$...$$ or \[...\]
 * Inline math: $...$ or \(...\)
 * Returns an array of React nodes.
 */
function processLatexSegments(text: string, textColor: string, accentColor: string): React.ReactNode[] {
  const segments = parseLatexSegments(text);
  const nodes: React.ReactNode[] = [];
  let key = 0;

  for (const seg of segments) {
    if (seg.type === 'text') {
      nodes.push(<Text key={`t${key++}`}>{seg.text}</Text>);
    } else if (seg.delimiter === 'display') {
      nodes.push(
        <View key={`math${key}`} style={styles.mathBlock}>
          <Text style={{ color: accentColor, fontStyle: 'italic', fontSize: 15, textAlign: 'center' }}>
            {seg.text}
          </Text>
        </View>
      );
    } else {
      nodes.push(
        <Text key={`math${key}`} style={{ color: accentColor, fontStyle: 'italic' }}>
          {seg.text}
        </Text>
      );
    }
    key++;
  }

  return nodes.length > 0 ? nodes : [<Text key="empty">{text}</Text>];
}

/** Lightweight markdown renderer — handles headers, bold, italic, code, lists, links, LaTeX, [[wikilinks]], auto-detected note refs. */
export default function MarkdownPreview({ content, textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap }: Props) {
  const lines = content.split('\n');
  const elements: React.ReactNode[] = [];
  let inCodeBlock = false;
  let codeLines: string[] = [];
  let codeKey = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // Fenced code block
    if (line.trimStart().startsWith('```')) {
      if (inCodeBlock) {
        elements.push(
          <View key={`code-${codeKey++}`} style={[styles.codeBlock, { backgroundColor: isDark ? '#1e1e1e' : '#f3f4f6' }]}>
            <ScrollView horizontal showsHorizontalScrollIndicator={false}>
              <Text style={{ color: textColor, fontSize: 13, fontFamily: 'monospace' }}>{codeLines.join('\n')}</Text>
            </ScrollView>
          </View>
        );
        codeLines = [];
        inCodeBlock = false;
      } else {
        inCodeBlock = true;
      }
      continue;
    }

    if (inCodeBlock) { codeLines.push(line); continue; }

    // Heading
    const headingMatch = line.match(/^(#{1,3})\s+(.*)/);
    if (headingMatch) {
      const level = headingMatch[1].length;
      elements.push(
        <Text key={`h-${i}`} style={[styles.heading, { fontSize: 24 - level * 3, color: textColor }]}>
          {headingMatch[2]}
        </Text>
      );
      continue;
    }

    // Horizontal rule
    if (/^(-{3,}|\*{3,}|_{3,})\s*$/.test(line)) {
      elements.push(<View key={`hr-${i}`} style={[styles.hr, { backgroundColor: isDark ? '#333' : '#ddd' }]} />);
      continue;
    }

    // Unordered list
    const listMatch = line.match(/^(\s*)[-*+]\s+(.*)/);
    if (listMatch) {
      elements.push(
        <Text key={`li-${i}`} style={[styles.paragraph, { color: textColor, paddingLeft: 8 + listMatch[1].length * 4 }]}>
          {'• '}{renderInline(listMatch[2], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap)}
        </Text>
      );
      continue;
    }

    // Ordered list
    const olMatch = line.match(/^(\s*)\d+\.\s+(.*)/);
    if (olMatch) {
      const num = line.match(/^\s*(\d+)\./)?.[1] ?? '1';
      elements.push(
        <Text key={`ol-${i}`} style={[styles.paragraph, { color: textColor, paddingLeft: 8 + olMatch[1].length * 4 }]}>
          {num}. {renderInline(olMatch[2], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap)}
        </Text>
      );
      continue;
    }

    // Empty line = spacing
    if (!line.trim()) {
      elements.push(<View key={`br-${i}`} style={{ height: 8 }} />);
      continue;
    }

    // Normal paragraph — with LaTeX processing
    const hasLatex = /\$/.test(line) || /\\\( /.test(line) || /\\\[/.test(line);
    if (hasLatex) {
      elements.push(
        <View key={`p-${i}`} style={{ flexDirection: 'row', flexWrap: 'wrap', marginBottom: 4 }}>
          {processLatexSegments(line, textColor, accentColor)}
        </View>
      );
    } else {
      elements.push(
        <Text key={`p-${i}`} style={[styles.paragraph, { color: textColor }]}>
          {renderInline(line, textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap)}
        </Text>
      );
    }
  }

  // Handle unclosed code blocks (e.g. stream interrupted, truncated output)
  if (inCodeBlock && codeLines.length > 0) {
    elements.push(
      <View key={`code-${codeKey++}`} style={[styles.codeBlock, { backgroundColor: isDark ? '#1e1e1e' : '#f3f4f6' }]}>
        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
          <Text style={{ color: textColor, fontSize: 13, fontFamily: 'monospace' }}>{codeLines.join('\n')}</Text>
        </ScrollView>
      </View>
    );
  }

  return <>{elements}</>;
}

/**
 * Parse inline markdown: **bold**, *italic*, `code`, [link](url), [[wikilink]].
 * Plus auto-detected note references (#2035) when noteTitleMap is provided.
 *
 * [[wikilink]] resolution: If onNoteLinkPress is provided, wikilinks render as
 * tappable underlined text with a note icon. Otherwise they render as plain
 * `[[title]]` text to avoid confusion.
 *
 * Priority: [[wikilink]] is checked before [link] so double-brackets don't
 * interfere with single-bracket patterns.
 */
function renderInline(
  text: string,
  textColor: string,
  accentColor: string,
  isDark: boolean,
  onNoteLinkPress?: (title: string) => void,
  noteTitleMap?: Map<string, string>,
): React.ReactNode {
  const parts: React.ReactNode[] = [];
  let remaining = text;
  let key = 0;

  while (remaining) {
    // Inline code (must check before brackets)
    const codeMatch = remaining.match(/^(.*?)`([^`]+)`(.*)$/);
    if (codeMatch) {
      if (codeMatch[1]) parts.push(...renderWithNoteRefs(codeMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      parts.push(
        <Text key={`c${key++}`} style={[styles.codeInline, { backgroundColor: isDark ? '#1e1e1e' : '#f3f4f6', color: textColor }]}>
          {codeMatch[2]}
        </Text>
      );
      remaining = codeMatch[3];
      continue;
    }

    // [[wikilink]] — must be checked before [link] to avoid double-bracket confusion
    const wikiMatch = remaining.match(/^(.*?)\[\[([^\[\]]+?)\]\]\s*(.*)$/);
    if (wikiMatch) {
      if (wikiMatch[1]) parts.push(...renderWithNoteRefs(wikiMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      if (onNoteLinkPress) {
        const noteTitle = wikiMatch[2].trim();
        parts.push(
          <TouchableOpacity
            key={`w${key++}`}
            onPress={() => onNoteLinkPress(noteTitle)}
            activeOpacity={0.6}
            style={styles.wikilinkTouch}
          >
            <View style={styles.wikilinkRow}>
              <Icon name="document-text-outline" size={13} color={accentColor} />
              <Text style={[styles.wikilinkText, { color: accentColor }]}>
                {noteTitle}
              </Text>
            </View>
          </TouchableOpacity>
        );
      } else {
        // No callback — render as plain text to avoid broken looking links
        parts.push(<Text key={`w${key++}`}>[[{wikiMatch[2]}]]</Text>);
      }
      remaining = wikiMatch[3];
      continue;
    }

    // Bold **text**
    const boldMatch = remaining.match(/^(.*?)\*\*([^*]+)\*\*(.*)$/);
    if (boldMatch) {
      if (boldMatch[1]) parts.push(...renderWithNoteRefs(boldMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      parts.push(<Text key={`b${key++}`} style={{ fontWeight: '700', color: textColor }}>{boldMatch[2]}</Text>);
      remaining = boldMatch[3];
      continue;
    }

    // Italic *text*
    const italicMatch = remaining.match(/^(.*?)\*([^*]+)\*(.*)$/);
    if (italicMatch) {
      if (italicMatch[1]) parts.push(...renderWithNoteRefs(italicMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      parts.push(<Text key={`i${key++}`} style={{ fontStyle: 'italic', color: textColor }}>{italicMatch[2]}</Text>);
      remaining = italicMatch[3];
      continue;
    }

    // Link [text](url)
    const linkMatch = remaining.match(/^(.*?)\[([^\]]+)\]\(([^)]+)\)(.*)$/);
    if (linkMatch) {
      if (linkMatch[1]) parts.push(...renderWithNoteRefs(linkMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      parts.push(<Text key={`l${key++}`} style={{ color: accentColor, textDecorationLine: 'underline' }}>{linkMatch[2]}</Text>);
      remaining = linkMatch[4];
      continue;
    }

    // Plain text — check for auto-detected note refs
    const nextSpecial = remaining.search(/[*`\[]/);
    if (nextSpecial > 0) {
      parts.push(...renderWithNoteRefs(remaining.slice(0, nextSpecial), textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      remaining = remaining.slice(nextSpecial);
    } else if (nextSpecial === -1) {
      parts.push(...renderWithNoteRefs(remaining, textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      remaining = '';
    } else {
      // Single special char — render it as text
      parts.push(...renderWithNoteRefs(remaining[0], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      remaining = remaining.slice(1);
    }
  }

  return parts.length === 1 ? parts[0] : <>{parts}</>;
}

/**
 * Render a plain text segment, splitting it into note reference links
 * when noteTitleMap is available and onNoteLinkPress is set.
 */
function renderWithNoteRefs(
  text: string,
  textColor: string,
  accentColor: string,
  isDark: boolean,
  onNoteLinkPress?: (title: string) => void,
  noteTitleMap?: Map<string, string>,
  key?: number,
): React.ReactNode[] {
  // No note map or callback → render as plain text
  if (!noteTitleMap || !onNoteLinkPress || !text) {
    return [<Text key={`t${key ?? 0}`}>{text}</Text>];
  }

  const refs = findNoteReferences(text, noteTitleMap);
  if (!refs.length) {
    return [<Text key={`t${key ?? 0}`}>{text}</Text>];
  }

  const segments = splitLineByNoteRefs(text, refs);
  return segments.map((seg, i) => {
    if (seg.isNoteRef) {
      return (
        <TouchableOpacity
          key={`nr${key ?? 0}-${i}`}
          onPress={() => onNoteLinkPress(seg.text)}
          activeOpacity={0.6}
          style={styles.wikilinkTouch}
        >
          <View style={styles.wikilinkRow}>
            <Icon name="document-text-outline" size={13} color={accentColor} />
            <Text style={[styles.wikilinkText, { color: accentColor }]}>
              {seg.text}
            </Text>
          </View>
        </TouchableOpacity>
      );
    }
    return <Text key={`t${key ?? 0}-${i}`}>{seg.text}</Text>;
  });
}

const styles = StyleSheet.create({
  heading: { fontWeight: '700', marginTop: 12, marginBottom: 6 },
  paragraph: { fontSize: 16, lineHeight: 24, marginBottom: 4 },
  codeBlock: { padding: 12, borderRadius: 8, marginVertical: 6 },
  codeInline: { paddingHorizontal: 4, borderRadius: 3, fontSize: 14, fontFamily: 'monospace' },
  hr: { height: 1, marginVertical: 8 },
  mathBlock: { backgroundColor: 'rgba(128,128,128,0.08)', borderRadius: 6, padding: 8, marginVertical: 6 },
  wikilinkTouch: {
    flexDirection: 'row',
    alignItems: 'center',
  },
  wikilinkRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 3,
  },
  wikilinkText: {
    fontSize: 16,
    lineHeight: 24,
    textDecorationLine: 'underline',
    fontWeight: '500',
  },
});
