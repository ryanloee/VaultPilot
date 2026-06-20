/**
 * Mobile RAG (Retrieval-Augmented Generation) service.
 * Searches local notes before chat and injects relevant context into prompts.
 * Also handles LLM-initiated note operations (save/search).
 */
import { searchNotes, createNote, updateNote, addTag, DbNote } from '../db';

/** Max notes to inject into context */
const MAX_CONTEXT_NOTES = 5;
/** Max chars per note content in context */
const MAX_NOTE_CONTENT_CHARS = 800;

/**
 * Extract keywords from user message for note search.
 * Strips common stop words and short tokens.
 */
function extractKeywords(text: string): string[] {
  const stopWords = new Set([
    '的', '了', '是', '在', '我', '有', '和', '就', '不', '人', '都', '一', '一个',
    '上', '也', '很', '到', '说', '要', '去', '你', '会', '着', '没有', '看', '好',
    '自己', '这', '他', '她', '它', '们', '那', '些', '什么', '怎么', '如何', '请',
    '帮', '我', '记录', '一下', '告诉', '知道', '可以', '能', '吗', '呢', '啊',
    'the', 'a', 'an', 'is', 'are', 'was', 'were', 'be', 'been', 'being',
    'have', 'has', 'had', 'do', 'does', 'did', 'will', 'would', 'shall',
    'should', 'may', 'might', 'can', 'could', 'i', 'you', 'he', 'she',
    'it', 'we', 'they', 'me', 'him', 'her', 'us', 'them', 'my', 'your',
    'his', 'its', 'our', 'their', 'this', 'that', 'these', 'those',
    'and', 'but', 'or', 'nor', 'not', 'so', 'yet', 'both', 'either',
    'neither', 'each', 'every', 'all', 'any', 'few', 'more', 'most',
    'other', 'some', 'such', 'no', 'only', 'own', 'same', 'than',
    'too', 'very', 'just', 'because', 'as', 'until', 'while', 'of',
    'at', 'by', 'for', 'with', 'about', 'against', 'between', 'through',
    'during', 'before', 'after', 'above', 'below', 'to', 'from', 'up',
    'down', 'in', 'out', 'on', 'off', 'over', 'under', 'again', 'further',
    'then', 'once', 'record', 'save', 'note', 'remember', 'please', 'hey',
  ]);

  // Split by whitespace and CJK boundaries, filter stop words and short tokens
  const tokens = text
    .split(/[\s,，。.!！?？;；:：、\n\r]+/)
    .flatMap(t => t.split(/(?<=[\u4e00-\u9fff])(?=[^\u4e00-\u9fff])|(?<=[^\u4e00-\u9fff])(?=[\u4e00-\u9fff])/))
    .map(t => t.trim().toLowerCase())
    .filter(t => t.length >= 2 && !stopWords.has(t));

  // Deduplicate
  return [...new Set(tokens)].slice(0, 10);
}

/**
 * Search notes relevant to the user's message and build context string.
 * Returns null if no relevant notes found.
 */
export async function buildNoteContext(userMessage: string): Promise<string | null> {
  try {
    const keywords = extractKeywords(userMessage);
    if (keywords.length === 0) return null;

    // Search with each keyword, collect unique results
    const seen = new Set<string>();
    const results: DbNote[] = [];

    for (const kw of keywords.slice(0, 5)) {
      const notes = await searchNotes(kw);
      for (const n of notes) {
        if (!seen.has(n.id) && results.length < MAX_CONTEXT_NOTES) {
          seen.add(n.id);
          results.push(n);
        }
      }
      if (results.length >= MAX_CONTEXT_NOTES) break;
    }

    if (results.length === 0) return null;

    // Build context block
    const blocks = results.map(n => {
      const title = n.title || '无标题';
      const content = n.content.length > MAX_NOTE_CONTENT_CHARS
        ? n.content.slice(0, MAX_NOTE_CONTENT_CHARS) + '...'
        : n.content;
      return `【${title}】\n${content}`;
    });

    return `以下是用户保存的可能相关的笔记：\n\n${blocks.join('\n\n---\n\n')}`;
  } catch (e) {
    console.warn('[RAG] Note search failed:', e);
    return null;
  }
}

/**
 * Parse LLM response for tool-call markers and execute them.
 * Supported markers:
 *   [SAVE_NOTE: title] content
 *   [TAG: noteTitle] tag1, tag2
 *
 * Returns the response with tool-call markers stripped,
 * plus a summary of actions taken.
 */
export async function executeToolCalls(
  response: string,
  onNoteSaved?: (title: string) => void,
): Promise<{ cleaned: string; actions: string[] }> {
  const actions: string[] = [];
  let cleaned = response;

  // Match [SAVE_NOTE: title] followed by content until next marker or end
  const savePattern = /\[SAVE_NOTE:\s*(.+?)\]\s*([\s\S]*?)(?=\[|$)/g;
  let match;
  const saves: { title: string; content: string }[] = [];

  while ((match = savePattern.exec(response)) !== null) {
    const title = match[1].trim();
    const content = match[2].trim();
    if (title && content) {
      saves.push({ title, content });
    }
  }

  for (const save of saves) {
    try {
      const noteId = await createNote(save.title);
      await updateNote(noteId, save.title, save.content);
      actions.push(`已保存笔记「${save.title}」`);
      onNoteSaved?.(save.title);
    } catch (e) {
      console.warn('[RAG] Failed to save note:', e);
      actions.push(`保存笔记「${save.title}」失败`);
    }
  }

  // Strip tool-call markers from displayed response
  cleaned = cleaned.replace(/\[SAVE_NOTE:[^\]]*\][\s\S]*?(?=\[|$)/g, '').trim();
  cleaned = cleaned.replace(/\[TAG:[^\]]*\]/g, '').trim();

  return { cleaned, actions };
}

/**
 * Build the system prompt with note-awareness instructions.
 */
export function buildSystemPrompt(noteContext: string | null): string {
  const base = `你是 VaultPilot AI 助手，知识渊博、乐于助人。用中文回答。`;

  const noteInstructions = `
你有以下能力：
1. 如果用户说"记录"、"保存"、"记下"等内容，使用 [SAVE_NOTE: 标题] 内容 的格式保存笔记。笔记内容要完整、结构化。
2. 回答时如果参考了用户的笔记，说明来源。
3. 不要在回复中显示标记本身，自然地融入回答中。`;

  let prompt = base;

  if (noteContext) {
    prompt += `\n\n${noteContext}`;
  }

  prompt += noteInstructions;

  return prompt;
}
