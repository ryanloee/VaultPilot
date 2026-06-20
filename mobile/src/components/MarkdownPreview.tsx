import React from 'react';
import { Text, View, StyleSheet } from 'react-native';

interface Props { content: string; textColor: string; accentColor: string; isDark: boolean; }

/** Lightweight markdown renderer — handles headers, bold, italic, code, lists, links. */
export default function MarkdownPreview({ content, textColor, accentColor, isDark }: Props) {
  const lines = content.split('\n');
  const elements: React.ReactNode[] = [];
  let inCodeBlock = false;
  let codeLines: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // Fenced code block
    if (line.trimStart().startsWith('```')) {
      if (inCodeBlock) {
        elements.push(
          <View key={`code-${i}`} style={[styles.codeBlock, { backgroundColor: isDark ? '#1e1e1e' : '#f3f4f6' }]}>
            <Text style={{ color: textColor, fontSize: 13, fontFamily: 'monospace' }}>{codeLines.join('\n')}</Text>
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
          {'• '}{renderInline(listMatch[2], textColor, accentColor)}
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
          {num}. {renderInline(olMatch[2], textColor, accentColor)}
        </Text>
      );
      continue;
    }

    // Empty line = spacing
    if (!line.trim()) {
      elements.push(<View key={`br-${i}`} style={{ height: 8 }} />);
      continue;
    }

    // Normal paragraph
    elements.push(
      <Text key={`p-${i}`} style={[styles.paragraph, { color: textColor }]}>
        {renderInline(line, textColor, accentColor)}
      </Text>
    );
  }

  return <>{elements}</>;
}

/** Parse inline markdown: **bold**, *italic*, `code`, [link](url) */
function renderInline(text: string, textColor: string, accentColor: string): React.ReactNode {
  // Split by inline patterns
  const parts: React.ReactNode[] = [];
  let remaining = text;
  let key = 0;

  while (remaining) {
    // Inline code
    const codeMatch = remaining.match(/^(.*?)`([^`]+)`(.*)$/);
    if (codeMatch) {
      if (codeMatch[1]) parts.push(...parseBold(codeMatch[1], textColor, accentColor, key));
      key += 10;
      parts.push(
        <Text key={`c${key++}`} style={[styles.codeInline, { backgroundColor: '#f3f4f6', color: textColor }]}>
          {codeMatch[2]}
        </Text>
      );
      remaining = codeMatch[3];
      continue;
    }

    // Bold **text**
    const boldMatch = remaining.match(/^(.*?)\*\*([^*]+)\*\*(.*)$/);
    if (boldMatch) {
      if (boldMatch[1]) parts.push(...parseBold(boldMatch[1], textColor, accentColor, key));
      key += 10;
      parts.push(<Text key={`b${key++}`} style={{ fontWeight: '700', color: textColor }}>{boldMatch[2]}</Text>);
      remaining = boldMatch[3];
      continue;
    }

    // Italic *text*
    const italicMatch = remaining.match(/^(.*?)\*([^*]+)\*(.*)$/);
    if (italicMatch) {
      if (italicMatch[1]) parts.push(...parseBold(italicMatch[1], textColor, accentColor, key));
      key += 10;
      parts.push(<Text key={`i${key++}`} style={{ fontStyle: 'italic', color: textColor }}>{italicMatch[2]}</Text>);
      remaining = italicMatch[3];
      continue;
    }

    // Link [text](url)
    const linkMatch = remaining.match(/^(.*?)\[([^\]]+)\]\(([^)]+)\)(.*)$/);
    if (linkMatch) {
      if (linkMatch[1]) parts.push(...parseBold(linkMatch[1], textColor, accentColor, key));
      key += 10;
      parts.push(<Text key={`l${key++}`} style={{ color: accentColor, textDecorationLine: 'underline' }}>{linkMatch[2]}</Text>);
      remaining = linkMatch[4];
      continue;
    }

    // Plain text — consume until next special char or end
    const nextSpecial = remaining.search(/[*`\[]/);
    if (nextSpecial > 0) {
      parts.push(<Text key={`t${key++}`}>{remaining.slice(0, nextSpecial)}</Text>);
      remaining = remaining.slice(nextSpecial);
    } else if (nextSpecial === -1) {
      parts.push(<Text key={`t${key++}`}>{remaining}</Text>);
      remaining = '';
    } else {
      // Special char at start but no match — consume one char
      parts.push(<Text key={`t${key++}`}>{remaining[0]}</Text>);
      remaining = remaining.slice(1);
    }
  }

  return parts.length === 1 ? parts[0] : <>{parts}</>;
}

function parseBold(text: string, textColor: string, accentColor: string, baseKey: number): React.ReactNode[] {
  // Simple passthrough — just return styled text
  return [<Text key={`t${baseKey}`}>{text}</Text>];
}

const styles = StyleSheet.create({
  heading: { fontWeight: '700', marginTop: 12, marginBottom: 6 },
  paragraph: { fontSize: 16, lineHeight: 24, marginBottom: 4 },
  codeBlock: { padding: 12, borderRadius: 8, marginVertical: 6 },
  codeInline: { paddingHorizontal: 4, borderRadius: 3, fontSize: 14, fontFamily: 'monospace' },
  hr: { height: 1, marginVertical: 8 },
});
