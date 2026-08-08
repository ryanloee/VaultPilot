import React, { memo, useState, useCallback, useMemo, useRef } from 'react';
import { Text, View, StyleSheet, ScrollView, TouchableOpacity, Image as RNImage } from 'react-native';
import { renderLatex, parseLatexSegments } from '../utils/latex';
import Icon from './Icon';
import { findNoteReferences, splitLineByNoteRefs } from '../utils/noteRefs';
import Lightbox from './Lightbox';
import MarkdownTable, { detectTable } from './MarkdownTable';
import MermaidDiagram from './MermaidDiagram';
import {
  extractImagesFromLine,
  extractStandaloneImages,
  isStandaloneImageLine,
  MarkdownImage,
  clamp,
} from '../utils/imageMarkdown';

interface Props {
  content: string;
  textColor: string;
  accentColor: string;
  isDark: boolean;
  /** Called when a [[wikilink]] or auto-detected note ref is tapped — receives the note title as argument. */
  onNoteLinkPress?: (title: string) => void;
  /** Map of note title (lowercase) → noteId for auto-detection of note references (#2035). */
  noteTitleMap?: Map<string, string>;
  /**
   * #3781 / #3963: called when the user deletes a selected standalone image
   * from the floating action bar — receives the 0-based line index (in the
   * original content string) of the image line to remove, so the parent can
   * delete exactly that one line even if duplicates exist. When omitted, the
   * 删除 button is hidden.
   */
  onDeleteImage?: (lineIndex: number) => void;
}

// ── #3781: image selection action bar (Obsidian 1.13 image interactions) ──
/** Base rendered width (px) of a standalone image before any resize. */
const DEFAULT_IMAGE_WIDTH = 300;
/** Each +/- press grows/shrinks the rendered width by 25% of the base. */
const IMAGE_WIDTH_STEP = 0.25;
const MIN_IMAGE_SCALE = 0.25;
const MAX_IMAGE_SCALE = 3;

interface ImageSelection {
  /** globalIdx of the selected image (aligned with the Lightbox index) */
  index: number;
  /**
   * #3963: 0-based index of the image's line in the ORIGINAL content string
   * (before footnote definitions are stripped). Passed to onDeleteImage so the
   * parent deletes exactly this one line, not all duplicate-text lines.
   */
  lineIndex: number;
  /** raw markdown line the image belongs to (kept for debugging/display) */
  line: string;
  /** single-image markdown syntax, e.g. ![alt](url "title") (copied) */
  markdown: string;
  /** rendered width scale; 1 = default width */
  scale: number;
}

/** Rebuild the markdown syntax for a single image (#3781 copy action). */
function imageMarkdown(img: MarkdownImage): string {
  return `![${img.alt}](${img.uri}${img.title ? ` "${img.title}"` : ''})`;
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

/** Lightweight markdown renderer — handles headers, bold, italic, code, lists, links, LaTeX, [[wikilinks]], auto-detected note refs.
 *
 * Supports ```mermaid code fences: renders them as a labelled diagram container
 * with a chart icon instead of a plain code block (#2805).
 */
const MarkdownPreview = memo(function MarkdownPreview({ content, textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, onDeleteImage }: Props) {
  // Collect standalone images for Lightbox navigation (#3030).
  // #3454: Must mirror the population rendered as tappable image blocks —
  // only standalone-line images get a `globalIdx` from `imageCounter`, so
  // `allImages` must contain only those images to keep indices aligned.
  // Inline images (e.g. `Hello ![emoji](e.png)`) are excluded.
  const allImages: MarkdownImage[] = useMemo(
    () => extractStandaloneImages(content),
    [content],
  );
  const [lightboxIndex, setLightboxIndex] = useState(-1);
  // #3781: long-press selection on standalone images (Obsidian 1.13 parity).
  const [selectedImage, setSelectedImage] = useState<ImageSelection | null>(null);
  // True between a long-press firing and that gesture's touchEnd — prevents
  // the touch area's onTouchEnd from instantly clearing a fresh selection.
  const longPressSuppressClearRef = useRef(false);
  const handleImagePress = useCallback((idx: number) => {
    setLightboxIndex(idx);
    setSelectedImage(null); // tapping an image (or anywhere) clears selection
  }, []);
  const handleCloseLightbox = useCallback(() => setLightboxIndex(-1), []);
  const handleIndexChange = useCallback((idx: number) => setLightboxIndex(idx), []);

  // ── #3781: selection / copy / delete / resize handlers ──────────────────
  const handleImageLongPress = useCallback((globalIdx: number, srcLineIndex: number, line: string, img: MarkdownImage) => {
    longPressSuppressClearRef.current = true;
    setSelectedImage((prev) =>
      prev && prev.index === globalIdx
        ? prev // re-long-pressing the same image keeps its zoom level
        : { index: globalIdx, lineIndex: srcLineIndex, line, markdown: imageMarkdown(img), scale: 1 },
    );
  }, []);

  const handleCopyImage = useCallback((markdown: string) => {
    // Lazy require: keeps expo-clipboard out of module scope — its ESM build
    // can't be transformed by this repo's ts-jest setup (tests mock it).
    const Clipboard = require('expo-clipboard') as typeof import('expo-clipboard');
    Clipboard.setStringAsync(markdown);
  }, []);

  const handleDeleteImage = useCallback(() => {
    if (!selectedImage) return;
    // #3963: pass the original line INDEX (not the line text) so the parent
    // deletes only this one occurrence — duplicate image lines are preserved.
    onDeleteImage?.(selectedImage.lineIndex);
    setSelectedImage(null);
  }, [selectedImage, onDeleteImage]);

  const handleResizeImage = useCallback((delta: number) => {
    setSelectedImage((prev) =>
      prev ? { ...prev, scale: clamp(prev.scale + delta, MIN_IMAGE_SCALE, MAX_IMAGE_SCALE) } : prev,
    );
  }, []);

  const handleResetImageSize = useCallback(() => {
    setSelectedImage((prev) => (prev ? { ...prev, scale: 1 } : prev));
  }, []);

  // Tap anywhere on the preview (empty area, text, or the end of a scroll
  // gesture) clears the selection — except for the tail of the long-press
  // gesture that just created it.
  const handleTouchAreaStart = useCallback(() => {
    longPressSuppressClearRef.current = false;
  }, []);
  const handleTouchAreaEnd = useCallback(() => {
    if (longPressSuppressClearRef.current) {
      longPressSuppressClearRef.current = false;
      return;
    }
    setSelectedImage(null);
  }, []);

  const lines = content.split('\n');
  const elements: React.ReactNode[] = [];
  let inCodeBlock = false;
  let codeLang: string | null = null;
  let codeLines: string[] = [];
  let codeKey = 0;
  // Global image counter for Lightbox index tracking (#3030)
  let imageCounter = 0;

  // ── First pass: collect footnote definitions (#3684) ──────────────────
  // GFM-style footnotes: [^id]: text
  // They appear as standalone lines (usually at the end) and should be
  // hidden from normal rendering. References ([^id] inline) are handled
  // in renderInline below.
  const footnoteDefs = new Map<string, string>();
  const ACTIVE_LINES: string[] = [];
  // #3963: Parallel array tracking each ACTIVE_LINES entry's original index in
  // content.split('\n'). Footnote definitions are dropped from ACTIVE_LINES, so
  // the render-loop index `i` does NOT equal the original line index — we need
  // this map to tell the parent exactly WHICH line to delete (by index, not by
  // text equality, which would delete all identical duplicate image lines).
  const activeSrcIndices: number[] = [];
  // #3697: The first pass must be fence-aware — a `[^id]: text` line inside a
  // ``` fenced code block is CODE content, not a footnote definition. Track
  // fence toggling exactly like the render loop below so code blocks keep
  // their lines (they previously rendered empty because the lines were
  // consumed as definitions).
  let inFence = false;
  for (let srcIdx = 0; srcIdx < lines.length; srcIdx++) {
    const rawLine = lines[srcIdx];
    if (rawLine.trimStart().startsWith('```')) {
      inFence = !inFence;
      ACTIVE_LINES.push(rawLine);
      activeSrcIndices.push(srcIdx);
      continue;
    }
    const trimmed = rawLine.trim();
    const defMatch = trimmed.match(/^\[\^([^\]]+)\]:[\t ](.*)$/);
    if (defMatch && !inFence) {
      const id = defMatch[1].trim();
      if (!footnoteDefs.has(id)) {
        footnoteDefs.set(id, defMatch[2].trim());
      }
      // Don't add to active lines — footnote definitions are hidden
    } else {
      ACTIVE_LINES.push(rawLine);
      activeSrcIndices.push(srcIdx);
    }
  }

  for (let i = 0; i < ACTIVE_LINES.length; i++) {
    const line = ACTIVE_LINES[i];

    // Fenced code block
    if (line.trimStart().startsWith('```')) {
      if (inCodeBlock) {
        const joined = codeLines.join('\n');
        // #3683: mermaid diagrams now render as actual SVG charts via WebView
        if (codeLang === 'mermaid') {
          elements.push(
            <MermaidDiagram
              key={`mermaid-${codeKey++}`}
              source={joined}
              accentColor={accentColor}
              textColor={textColor}
              isDark={isDark}
            />
          );
        } else {
          elements.push(
            <View key={`code-${codeKey++}`} style={[styles.codeBlock, { backgroundColor: isDark ? '#1e1e1e' : '#f3f4f6' }]}>
              <ScrollView horizontal showsHorizontalScrollIndicator={false}>
                <Text style={{ color: textColor, fontSize: 13, fontFamily: 'monospace' }}>{joined}</Text>
              </ScrollView>
            </View>
          );
        }
        codeLines = [];
        codeLang = null;
        inCodeBlock = false;
      } else {
        inCodeBlock = true;
        // Extract language hint: ```mermaid, ```rust, ```python, etc.
        const langHint = line.trimStart().slice(3).trim();
        codeLang = langHint || null;
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

    // Markdown table (#3685): detect pipe-delimited table rows
    {
      const tableLineCount = detectTable(ACTIVE_LINES, i);
      if (tableLineCount >= 2) {
        const tableLines = ACTIVE_LINES.slice(i, i + tableLineCount);
        elements.push(
          <MarkdownTable
            key={`table-${i}`}
            lines={tableLines}
            textColor={textColor}
            accentColor={accentColor}
            isDark={isDark}
          />
        );
        i += tableLineCount - 1; // skip consumed lines
        continue;
      }
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

    // Image line (#3030): render ![alt](url) as clickable images that open Lightbox
    const lineImages = extractImagesFromLine(line);
    if (lineImages.length > 0 && isStandaloneImageLine(line)) {
      // #3963: original line index in the source content (ACTIVE_LINES may
      // have dropped footnote definitions, so `i` ≠ source index).
      const srcLineIndex = activeSrcIndices[i];
      const imgElements = lineImages.map((img, imgIdx) => {
        const globalIdx = imageCounter++;
        // #3781: long-press selects the image; the selected image gets a blue
        // border and (when resized) an explicit pixel width.
        const isSelected = selectedImage?.index === globalIdx;
        const resizedStyle = isSelected && selectedImage.scale !== 1
          ? { width: Math.round(DEFAULT_IMAGE_WIDTH * selectedImage.scale) }
          : null;
        return (
          <TouchableOpacity
            key={`img-${i}-${imgIdx}`}
            activeOpacity={0.85}
            onPress={() => handleImagePress(globalIdx)}
            onLongPress={() => handleImageLongPress(globalIdx, srcLineIndex, line, img)}
            testID={`md-image-${globalIdx}`}
          >
            <RNImage
              source={{ uri: img.uri }}
              style={[
                styles.inlineImage,
                lineImages.length > 1 && styles.inlineImageGrid,
                isSelected && styles.imageSelected,
                resizedStyle,
              ]}
              resizeMode="cover"
              accessibilityLabel={img.alt || 'Image'}
            />
          </TouchableOpacity>
        );
      });

      elements.push(
        <View key={`imgblock-${i}`} style={styles.imageBlock}>
          {imgElements}
        </View>
      );
      continue;
    }

    // Empty line = spacing
    if (!line.trim()) {
      elements.push(<View key={`br-${i}`} style={{ height: 8 }} />);
      continue;
    }

    // Normal paragraph — with LaTeX processing
    const hasLatex = /\$/.test(line) || /\\\(/.test(line) || /\\\[/.test(line);
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

  // Handle unclosed code blocks / mermaid diagrams (e.g. stream interrupted, truncated output)
  if (inCodeBlock && codeLines.length > 0) {
    const joined = codeLines.join('\n');
    // #3683: unclosed mermaid blocks also render via MermaidDiagram
    if (codeLang === 'mermaid') {
      elements.push(
        <MermaidDiagram
          key={`mermaid-${codeKey++}`}
          source={joined}
          accentColor={accentColor}
          textColor={textColor}
          isDark={isDark}
        />
      );
    } else {
      elements.push(
        <View key={`code-${codeKey++}`} style={[styles.codeBlock, { backgroundColor: isDark ? '#1e1e1e' : '#f3f4f6' }]}>
          <ScrollView horizontal showsHorizontalScrollIndicator={false}>
            <Text style={{ color: textColor, fontSize: 13, fontFamily: 'monospace' }}>{joined}</Text>
          </ScrollView>
        </View>
      );
    }
  }

  // ── Render footnote definitions at end (#3684) ──────────────────────
  if (footnoteDefs.size > 0) {
    // Separator line
    elements.push(
      <View key="fn-sep" style={[styles.hr, { backgroundColor: isDark ? 'rgba(255,255,255,0.12)' : 'rgba(0,0,0,0.1)' }]} />
    );
    const sortedIds = [...footnoteDefs.keys()].sort((a, b) => {
      const an = parseInt(a, 10);
      const bn = parseInt(b, 10);
      if (!isNaN(an) && !isNaN(bn)) return an - bn;
      return a.localeCompare(b);
    });
    for (const id of sortedIds) {
      const text = footnoteDefs.get(id)!;
      elements.push(
        <View key={`fn-def-${id}`} style={styles.footnoteItem}>
          <Text style={[styles.footnoteId, { color: accentColor }]}>[{id}]</Text>
          <Text style={[styles.footnoteText, { color: textColor }]}>{renderInline(text, textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap)}</Text>
        </View>
      );
    }
  }

  return (
    <View style={styles.previewRoot}>
      {/* Touch area: taps outside images/text (or scroll gesture ends) clear the selection (#3781) */}
      <View
        testID="md-preview-touch-area"
        onTouchStart={handleTouchAreaStart}
        onTouchEnd={handleTouchAreaEnd}
      >
        {elements}
      </View>
      {/* #3781: floating action bar for the selected image (Obsidian 1.13 parity) */}
      {selectedImage && (
        <View testID="md-image-action-bar" style={styles.imageActionBar}>
          <TouchableOpacity
            testID="md-image-copy-btn"
            style={styles.imageActionBtn}
            onPress={() => handleCopyImage(selectedImage.markdown)}
          >
            <Text style={styles.imageActionBtnText}>复制</Text>
          </TouchableOpacity>
          {onDeleteImage && (
            <TouchableOpacity
              testID="md-image-delete-btn"
              style={styles.imageActionBtn}
              onPress={handleDeleteImage}
            >
              <Text style={styles.imageActionBtnText}>删除</Text>
            </TouchableOpacity>
          )}
          <TouchableOpacity
            testID="md-image-zoom-out"
            style={styles.imageActionBtn}
            onPress={() => handleResizeImage(-IMAGE_WIDTH_STEP)}
          >
            <Text style={styles.imageActionBtnText}>-</Text>
          </TouchableOpacity>
          <TouchableOpacity
            testID="md-image-zoom-in"
            style={styles.imageActionBtn}
            onPress={() => handleResizeImage(IMAGE_WIDTH_STEP)}
          >
            <Text style={styles.imageActionBtnText}>+</Text>
          </TouchableOpacity>
          <TouchableOpacity
            testID="md-image-zoom-reset"
            style={styles.imageActionBtn}
            onPress={handleResetImageSize}
          >
            <Text style={styles.imageActionBtnText}>0</Text>
          </TouchableOpacity>
        </View>
      )}
      {allImages.length > 0 && lightboxIndex >= 0 && (
        <Lightbox
          visible={lightboxIndex >= 0}
          images={allImages}
          index={Math.min(lightboxIndex, allImages.length - 1)}
          onClose={handleCloseLightbox}
          onIndexChange={handleIndexChange}
        />
      )}
    </View>
  );
});

export default MarkdownPreview;

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

    // Bold **text** — rendered via a nested pass so footnote refs ([^id])
    // inside the span still become superscripts without destroying the
    // `**` delimiters (#3690, #3695).
    const boldMatch = remaining.match(/^(.*?)\*\*([^*]+)\*\*(.*)$/);
    if (boldMatch) {
      if (boldMatch[1]) parts.push(...renderSpanWithFnRefs(boldMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      parts.push(<Text key={`b${key++}`} style={{ fontWeight: '700', color: textColor }}>{renderSpanWithFnRefs(boldMatch[2], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key)}</Text>);
      remaining = boldMatch[3];
      continue;
    }

    // Italic *text* — same nested fn-ref pass as bold (#3690, #3695)
    const italicMatch = remaining.match(/^(.*?)\*([^*]+)\*(.*)$/);
    if (italicMatch) {
      if (italicMatch[1]) parts.push(...renderSpanWithFnRefs(italicMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      parts.push(<Text key={`i${key++}`} style={{ fontStyle: 'italic', color: textColor }}>{renderSpanWithFnRefs(italicMatch[2], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key)}</Text>);
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

    // Footnote reference [^id] (#3684) — must be AFTER [link](url) so a
    // valid GFM link with text `^1` ([^1](url)) is not eaten by the fn ref
    // matcher (#3695). Also after bold/italic — refs inside those spans are
    // handled by renderSpanWithFnRefs in the branches above.
    const fnMatch = remaining.match(/^(.*?)\[\^([^\]]+)\](.*)$/);
    if (fnMatch) {
      if (fnMatch[1]) parts.push(...renderWithNoteRefs(fnMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      // Render as superscript reference (e.g. [1], [note])
      parts.push(
        <Text key={`fn${key++}`} style={{ fontSize: 12, color: accentColor, fontWeight: '600', lineHeight: 20 }}>
          [{fnMatch[2]}]
        </Text>
      );
      remaining = fnMatch[3];
      continue;
    }

    // Plain text — check for auto-detected note refs
    const nextSpecial = remaining.search(/[*`\[]/);
    if (nextSpecial > 0) {
      parts.push(...renderWithNoteRefs(remaining.slice(0, nextSpecial), textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      key++;
      remaining = remaining.slice(nextSpecial);
    } else if (nextSpecial === -1) {
      parts.push(...renderWithNoteRefs(remaining, textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      key++;
      remaining = '';
    } else {
      // Single special char — render it as text
      parts.push(...renderWithNoteRefs(remaining[0], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, key));
      key++;
      remaining = remaining.slice(1);
    }
  }

  return parts.length === 1 ? parts[0] : <>{parts}</>;
}

/**
 * Render a text segment, converting footnote refs ([^id]) to superscripts
 * while passing everything else through renderWithNoteRefs (plain text +
 * auto-detected note refs). Used for bold/italic span content AND their
 * prefixes, so `**bold [^1]**` keeps its bold formatting and the ref still
 * becomes a superscript (#3690, #3695).
 */
function renderSpanWithFnRefs(
  text: string,
  textColor: string,
  accentColor: string,
  isDark: boolean,
  onNoteLinkPress?: (title: string) => void,
  noteTitleMap?: Map<string, string>,
  key?: number,
): React.ReactNode[] {
  const parts: React.ReactNode[] = [];
  let remaining = text;
  let k = key ?? 0;
  while (remaining) {
    const fnMatch = remaining.match(/^(.*?)\[\^([^\]]+)\](.*)$/);
    if (fnMatch) {
      if (fnMatch[1]) parts.push(...renderWithNoteRefs(fnMatch[1], textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, k));
      parts.push(
        <Text key={`fn${k++}`} style={{ fontSize: 12, color: accentColor, fontWeight: '600', lineHeight: 20 }}>
          [{fnMatch[2]}]
        </Text>
      );
      remaining = fnMatch[3];
      continue;
    }
    parts.push(...renderWithNoteRefs(remaining, textColor, accentColor, isDark, onNoteLinkPress, noteTitleMap, k));
    break;
  }
  return parts;
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
  // #3683: Mermaid card styles moved to MermaidDiagram.tsx component
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
  // #3030: Image rendering styles
  imageBlock: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    justifyContent: 'center',
    marginVertical: 8,
    gap: 4,
  },
  inlineImage: {
    width: '100%',
    height: 200,
    borderRadius: 8,
  },
  inlineImageGrid: {
    width: '48%',
    height: 140,
  },
  // #3781: image selection + floating action bar (Obsidian 1.13 parity)
  previewRoot: {
    position: 'relative',
  },
  imageSelected: {
    borderWidth: 2,
    borderColor: '#3b82f6',
  },
  imageActionBar: {
    position: 'absolute',
    bottom: 8,
    left: 16,
    right: 16,
    flexDirection: 'row',
    justifyContent: 'center',
    alignItems: 'center',
    gap: 8,
    paddingVertical: 8,
    paddingHorizontal: 12,
    borderRadius: 12,
    backgroundColor: 'rgba(24, 24, 27, 0.92)',
    elevation: 4,
    shadowColor: '#000',
    shadowOpacity: 0.25,
    shadowRadius: 6,
    shadowOffset: { width: 0, height: 2 },
  },
  imageActionBtn: {
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderRadius: 8,
    backgroundColor: 'rgba(255, 255, 255, 0.16)',
  },
  imageActionBtnText: {
    color: '#fff',
    fontSize: 14,
    fontWeight: '600',
  },
  // #3684: Footnote rendering
  footnoteItem: {
    flexDirection: 'row',
    marginBottom: 6,
    paddingRight: 8,
  },
  footnoteId: {
    fontSize: 13,
    fontWeight: '600',
    marginRight: 6,
    lineHeight: 22,
  },
  footnoteText: {
    fontSize: 14,
    lineHeight: 22,
    flex: 1,
  },
});
