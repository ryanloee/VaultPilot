import React from 'react';
import { Text, View, StyleSheet, ScrollView, Pressable } from 'react-native';
import { renderLatex, parseLatexSegments } from '../utils/latex';

interface Props {
  content: string;
  textColor: string;
  accentColor: string;
  isDark: boolean;
  /** Called when a [[wikilink]] or [[note#^blockid]] is tapped */
  onNoteLinkPress?: (noteName: string, blockId?: string) => void;
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

/** Lightweight markdown renderer — handles headers, bold, italic, code, lists, links, LaTeX, wikilinks, blockrefs. */
export default function MarkdownPreview({ content, textColor, accentColor, isDark, onNoteLinkPress }: Props) {
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
          {'• '}{renderInline(listMatch[2], textColor, accentColor, isDark, onNoteLinkPress)}
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
          {num}. {renderInline(olMatch[2], textColor, accentColor, isDark, onNoteLinkPress)}
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
          {renderInline(line, textColor, accentColor, isDark, onNoteLinkPress)}
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
 * Parse inline markdown: **bold**, *italic*, `code`, [link](url),
 * [[wikilink]], [[note#^blockid|display]].
 */
function renderInline(
  text: string,
  textColor: string,
  accentColor: string,
  isDark: boolean,
  onNoteLinkPress?: (noteName: string, blockId?: string) => void,
): React.ReactNode {
  const parts: React.ReactNode[] = [];
  let remaining = text;
  let key = 0;

  while (remaining) {
    // Inline code
    const codeMatch = remaining.match(/^(.*?)`([^`]+)`(.*)$/);
    if (codeMatch) {
      if (codeMatch[1]) parts.push(...[<Text key={`t${key++}`}>{codeMatch[1]}</Text>]);
      parts.push(
        <Text key={`c${key++}`} style={[styles.codeInline, { backgroundColor: isDark ? '#1e1e1e' : '#f3f4f6', color: textColor }]}>
          {codeMatch[2]}
        </Text>
      );
      remaining = codeMatch[3];
      continue;
    }

    // Wikilink / Block reference [[note#^blockid]] or [[Note Name]] or [[Note Name|Display]]
    const wikilinkMatch = remaining.match(
      /^(.*?)\[\[([^\[\]]+?)(?:\|([^\[\]]*?))?\]\](.*)$/,
    );
    if (wikilinkMatch) {
      if (wikilinkMatch[1]) parts.push(<Text key={`t${key++}`}>{wikilinkMatch[1]}</Text>);
      const raw = wikilinkMatch[2];
      const display = wikilinkMatch[3] || raw;
      const caretPos = raw.indexOf('#^');
      const hashPos = raw.indexOf('#');
      const isBlockRef = caretPos >= 0 || (hashPos >= 0 && raw.includes('#^', 0)) || (hashPos >= 0 && !raw.includes('[['));
      const linkStyle = { color: accentColor, textDecorationLine: 'underline' as const };
      if (caretPos >= 0) {
        // Block reference: [[Note Name#^blockid]]
        const noteName = caretPos === 0 ? undefined : raw.slice(0, caretPos).trim();
        const blockId = raw.slice(caretPos + 2).trim();
        parts.push(
          <Pressable
            key={`wl${key++}`}
            onPress={() => {
              onNoteLinkPress?.(noteName || '', `^${blockId}`);
            }}
          >
            <Text style={linkStyle}>{display}</Text>
          </Pressable>
        );
      } else if (hashPos >= 0) {
        // Heading anchor: [[Note Name#Heading]]
        const noteName = hashPos === 0 ? undefined : raw.slice(0, hashPos).trim();
        const heading = raw.slice(hashPos + 1).trim();
        parts.push(
          <Pressable
            key={`wl${key++}`}
            onPress={() => {
              onNoteLinkPress?.(noteName || '', heading);
            }}
          >
            <Text style={linkStyle}>{display}</Text>
          </Pressable>
        );
      } else {
        // Simple wikilink: [[Note Name]]
        parts.push(
          <Pressable
            key={`wl${key++}`}
            onPress={() => {
              onNoteLinkPress?.(raw.trim());
            }}
          >
            <Text style={linkStyle}>{display}</Text>
          </Pressable>
        );
      }
      remaining = wikilinkMatch[4];
      continue;
    }

    // Bold **text**
    const boldMatch = remaining.match(/^(.*?)\*\*([^*]+)\*\*(.*)$/);
    if (boldMatch) {
      if (boldMatch[1]) parts.push(<Text key={`t${key++}`}>{boldMatch[1]}</Text>);
      parts.push(<Text key={`b${key++}`} style={{ fontWeight: '700', color: textColor }}>{boldMatch[2]}</Text>);
      remaining = boldMatch[3];
      continue;
    }

    // Italic *text*
    const italicMatch = remaining.match(/^(.*?)\*([^*]+)\*(.*)$/);
    if (italicMatch) {
      if (italicMatch[1]) parts.push(<Text key={`t${key++}`}>{italicMatch[1]}</Text>);
      parts.push(<Text key={`i${key++}`} style={{ fontStyle: 'italic', color: textColor }}>{italicMatch[2]}</Text>);
      remaining = italicMatch[3];
      continue;
    }

    // Link [text](url)
    const linkMatch = remaining.match(/^(.*?)\[([^\]]+)\]\(([^)]+)\)(.*)$/);
    if (linkMatch) {
      if (linkMatch[1]) parts.push(<Text key={`t${key++}`}>{linkMatch[1]}</Text>);
      parts.push(<Text key={`l${key++}`} style={{ color: accentColor, textDecorationLine: 'underline' }}>{linkMatch[2]}</Text>);
      remaining = linkMatch[4];
      continue;
    }

    // Plain text
    const nextSpecial = remaining.search(/[*`\[]/);
    if (nextSpecial > 0) {
      parts.push(<Text key={`t${key++}`}>{remaining.slice(0, nextSpecial)}</Text>);
      remaining = remaining.slice(nextSpecial);
    } else if (nextSpecial === -1) {
      parts.push(<Text key={`t${key++}`}>{remaining}</Text>);
      remaining = '';
    } else {
      parts.push(<Text key={`t${key++}`}>{remaining[0]}</Text>);
      remaining = remaining.slice(1);
    }
  }

  return parts.length === 1 ? parts[0] : <>{parts}</>;
}

const styles = StyleSheet.create({
  heading: { fontWeight: '700', marginTop: 12, marginBottom: 6 },
  paragraph: { fontSize: 16, lineHeight: 24, marginBottom: 4 },
  codeBlock: { padding: 12, borderRadius: 8, marginVertical: 6 },
  codeInline: { paddingHorizontal: 4, borderRadius: 3, fontSize: 14, fontFamily: 'monospace' },
  hr: { height: 1, marginVertical: 8 },
  mathBlock: { backgroundColor: 'rgba(128,128,128,0.08)', borderRadius: 6, padding: 8, marginVertical: 6 },
});
