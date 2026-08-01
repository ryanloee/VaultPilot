/**
 * Markdown Table Renderer (#3685).
 *
 * Renders GFM-style pipe-delimited tables as styled card-grid layouts.
 * Supports header row, alignment, and column auto-sizing.
 *
 * Table syntax:
 *   | Header 1 | Header 2 |
 *   |----------|:--------:|
 *   | Cell 1   | Cell 2   |
 */

import React from 'react';
import { View, Text, StyleSheet, ScrollView } from 'react-native';

export interface MarkdownTableProps {
  /** Raw table source lines (each line is a `|...|` row) */
  lines: string[];
  /** Text color for cell content */
  textColor: string;
  /** Accent color for header background */
  accentColor: string;
  /** Whether the app is in dark mode */
  isDark: boolean;
}

type Alignment = 'left' | 'center' | 'right';

interface ParsedTable {
  headers: string[];
  alignments: Alignment[];
  rows: string[][];
}

/**
 * Parse a markdown separator row (e.g., `|:---|:---:|---:|`)
 * into per-column alignment values.
 */
function parseAlignments(separator: string): Alignment[] {
  const cells = splitTableRow(separator);
  return cells.map((cell) => {
    const trimmed = cell.trim();
    const leftColon = trimmed.startsWith(':');
    const rightColon = trimmed.endsWith(':');
    if (leftColon && rightColon) return 'center';
    if (rightColon) return 'right';
    return 'left';
  });
}

/**
 * Split a `|`-delimited row into individual cell contents.
 * Strips leading/trailing pipes and trims whitespace.
 */
function splitTableRow(line: string): string[] {
  // Remove leading/trailing pipe and whitespace
  let trimmed = line.trim();
  if (trimmed.startsWith('|')) trimmed = trimmed.slice(1);
  if (trimmed.endsWith('|')) trimmed = trimmed.slice(0, -1);
  return trimmed.split('|').map((c) => c.trim());
}

/**
 * Detect if a line is a table separator row.
 * Valid separators: `---`, `:--`, `--:`, `:--:`, etc.
 */
function isSeparatorRow(line: string): boolean {
  const cells = splitTableRow(line);
  if (cells.length === 0) return false;
  return cells.every((cell) => {
    const trimmed = cell.trim();
    if (!trimmed) return false;
    return /^:?-{2,}:?$/.test(trimmed) || /^:?-+:?$/.test(trimmed);
  });
}

/**
 * Detect if a line looks like a table row (has at least 2 pipe chars).
 */
function isTableRow(line: string): boolean {
  const pipeCount = (line.match(/\|/g) || []).length;
  return pipeCount >= 2;
}

/**
 * Parse multiple lines into a structured table.
 * Returns null if the lines don't form a valid table.
 */
export function parseMarkdownTable(lines: string[]): ParsedTable | null {
  if (lines.length < 2) return null;

  // First line = header
  const headers = splitTableRow(lines[0]);
  if (headers.length < 1) return null;

  // Second line = separator (required for GFM tables)
  if (!isSeparatorRow(lines[1])) return null;
  const alignments = parseAlignments(lines[1]);

  // Pad alignments to match header count
  while (alignments.length < headers.length) {
    alignments.push('left');
  }

  // Remaining lines = data rows
  const rows: string[][] = [];
  for (let i = 2; i < lines.length; i++) {
    const row = splitTableRow(lines[i]);
    if (row.length < 1) continue;
    rows.push(row);
  }

  return { headers, alignments, rows };
}

/**
 * Check if a line sequence starting at index 0 forms a table.
 * Returns the number of lines consumed, or 0 if not a table.
 */
export function detectTable(lines: string[], startIndex: number): number {
  if (startIndex + 1 >= lines.length) return 0;
  if (!isTableRow(lines[startIndex])) return 0;
  if (!isSeparatorRow(lines[startIndex + 1])) return 0;

  // Count consecutive table rows
  let count = startIndex + 2;
  while (count < lines.length && isTableRow(lines[count])) {
    count++;
  }
  return count - startIndex;
}

function getAlignStyle(align: Alignment): { textAlign: 'left' | 'center' | 'right' } {
  return { textAlign: align };
}

export default function MarkdownTable({
  lines,
  textColor,
  accentColor,
  isDark,
}: MarkdownTableProps) {
  const parsed = parseMarkdownTable(lines);
  if (!parsed) {
    // Fallback: render as code block
    return (
      <View style={[styles.fallback, { backgroundColor: isDark ? '#1e1e1e' : '#f3f4f6' }]}>
        <ScrollView horizontal showsHorizontalScrollIndicator={false}>
          <Text style={{ color: textColor, fontSize: 13, fontFamily: 'monospace' }}>
            {lines.join('\n')}
          </Text>
        </ScrollView>
      </View>
    );
  }

  const { headers, alignments, rows } = parsed;
  const numCols = headers.length;
  const headerBg = isDark ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.03)';
  const borderColor = isDark ? 'rgba(255,255,255,0.1)' : 'rgba(0,0,0,0.08)';
  const altRowBg = isDark ? 'rgba(255,255,255,0.02)' : 'rgba(0,0,0,0.01)';

  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      style={styles.scrollContainer}
      testID="md-table-scroll"
    >
      <View style={[styles.tableContainer, { borderColor }]} testID="md-table">
        {/* Header row */}
        <View style={[styles.row, { backgroundColor: headerBg }]}>
          {headers.map((header, colIdx) => (
            <View
              key={`th-${colIdx}`}
              style={[styles.cell, { borderRightColor: borderColor, minWidth: 80 }]}
            >
              <Text
                style={[
                  styles.headerText,
                  { color: accentColor },
                  getAlignStyle(alignments[colIdx] || 'left'),
                ]}
              >
                {header}
              </Text>
            </View>
          ))}
        </View>

        {/* Data rows */}
        {rows.map((row, rowIdx) => (
          <View
            key={`tr-${rowIdx}`}
            style={[
              styles.row,
              {
                backgroundColor: rowIdx % 2 === 1 ? altRowBg : 'transparent',
                borderTopColor: borderColor,
              },
            ]}
          >
            {Array.from({ length: numCols }).map((_, colIdx) => {
              const cellText = row[colIdx] || '';
              return (
                <View
                  key={`td-${rowIdx}-${colIdx}`}
                  style={[styles.cell, { borderRightColor: borderColor, minWidth: 80 }]}
                >
                  <Text
                    style={[
                      styles.cellText,
                      { color: textColor },
                      getAlignStyle(alignments[colIdx] || 'left'),
                    ]}
                  >
                    {cellText}
                  </Text>
                </View>
              );
            })}
          </View>
        ))}
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scrollContainer: {
    marginVertical: 8,
  },
  tableContainer: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 6,
    overflow: 'hidden',
  },
  row: {
    flexDirection: 'row',
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  cell: {
    paddingVertical: 6,
    paddingHorizontal: 10,
    borderRightWidth: StyleSheet.hairlineWidth,
    flexShrink: 0,
  },
  headerText: {
    fontSize: 13,
    fontWeight: '700',
  },
  cellText: {
    fontSize: 14,
    lineHeight: 20,
  },
  fallback: {
    padding: 12,
    borderRadius: 8,
    marginVertical: 6,
  },
});
