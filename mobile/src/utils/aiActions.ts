/**
 * AI Command Palette — action registry & message builders (#2188 Phase 1).
 *
 * Pure, side-effect-free helpers so the message-building logic is unit-testable
 * independent of React Native / streaming. Consumed by AiCommandPalette.tsx.
 */

import { ChatMessage } from '../api/client';

export type AiActionId =
  | 'summarize'
  | 'rewrite'
  | 'translate_en'
  | 'translate_zh'
  | 'continue'
  | 'explain'
  | 'extract_todos'
  | 'custom';

export interface AiAction {
  id: AiActionId;
  label: string;
  /** Ionicons glyph name (no emoji per project convention). */
  icon: string;
  description: string;
  /** Keywords used by the fuzzy filter (lower-cased). */
  keywords: string[];
  /** When true the action requires a free-form user instruction. */
  needsUserPrompt?: boolean;
}

/**
 * MVP action set. Each entry is atomic (no sub-flows) so the palette UX stays a
 * single tap. Parameterised variants (e.g. translate EN vs ZH) are split into
 * separate concrete actions.
 */
export const AI_ACTIONS: AiAction[] = [
  {
    id: 'summarize',
    label: '总结要点',
    icon: 'bullets-outline',
    description: '将选中文本或笔记精简为要点摘要',
    keywords: ['summarize', '总结', '摘要', '要点', 'summarise', 'tldr'],
  },
  {
    id: 'rewrite',
    label: '改写润色',
    icon: 'create-outline',
    description: '改善清晰度、流畅度与表达',
    keywords: ['rewrite', '改写', '润色', 'polish', 'improve', '重写'],
  },
  {
    id: 'translate_en',
    label: '翻译为英文',
    icon: 'language-outline',
    description: '将选中文本翻译为英文',
    keywords: ['translate', '翻译', 'english', '英文', 'en'],
  },
  {
    id: 'translate_zh',
    label: '翻译为中文',
    icon: 'language-outline',
    description: '将选中文本翻译为简体中文',
    keywords: ['translate', '翻译', 'chinese', '中文', 'zh'],
  },
  {
    id: 'continue',
    label: '续写',
    icon: 'arrow-forward-outline',
    description: '基于当前内容继续往下写',
    keywords: ['continue', '续写', '扩写', 'extend', '写下去'],
  },
  {
    id: 'explain',
    label: '解释说明',
    icon: 'bulb-outline',
    description: '解释选中的概念或术语',
    keywords: ['explain', '解释', '说明', '概念', '术语'],
  },
  {
    id: 'extract_todos',
    label: '提取待办',
    icon: 'checkbox-outline',
    description: '从内容中提取 Action Items 列表',
    keywords: ['todo', '待办', 'action', '任务', '提取', 'todos'],
  },
  {
    id: 'custom',
    label: '自定义写作',
    icon: 'sparkles-outline',
    description: '输入自定义指令让 AI 帮你写',
    keywords: ['custom', '自定义', '写作', 'write', 'ai', '自由'],
    needsUserPrompt: true,
  },
];

/**
 * Decide what text an action operates on: a non-empty selection wins, otherwise
 * the whole note. Exported for testing and reuse by the editor.
 */
export function resolveContext(selection: string, noteContent: string): string {
  const sel = (selection ?? '').trim();
  if (sel.length > 0) return selection.trim();
  return (noteContent ?? '').trim();
}

/** Whether the action has any non-empty target text to work on. */
export function hasContext(selection: string, noteContent: string): boolean {
  return resolveContext(selection, noteContent).length > 0;
}

const COMMON_RULES =
  'Respond with only the resulting content. Do not add conversational commentary, prefaces, or formatting instructions. Preserve the original meaning.';

function contextBlock(text: string): string {
  return `<context>\n${text}\n</context>`;
}

/**
 * Build the ChatMessage array for a given action. Pure function — same inputs
 * always yield the same messages.
 */
export function buildActionMessages(
  actionId: AiActionId,
  context: string,
  userPrompt?: string,
): ChatMessage[] {
  const ctx = contextBlock(context);

  switch (actionId) {
    case 'summarize':
      return [
        {
          role: 'system',
          content:
            'You are a concise summarization assistant inside a note-taking app. Produce a tight bullet-point summary of the provided content. Use the same language as the source. ' +
            COMMON_RULES,
        },
        {
          role: 'user',
          content: `Summarize the following content as key bullet points:\n\n${ctx}`,
        },
      ];

    case 'rewrite':
      return [
        {
          role: 'system',
          content:
            'You are a professional editing assistant inside a note-taking app. Rewrite the provided content to improve clarity, flow, and readability while preserving meaning and the original language. Keep the same structure unless it harms clarity. ' +
            COMMON_RULES,
        },
        {
          role: 'user',
          content: `Rewrite and polish the following content:\n\n${ctx}`,
        },
      ];

    case 'translate_en':
      return [
        {
          role: 'system',
          content:
            'You are a translation assistant inside a note-taking app. Translate the provided content into natural, fluent English. Preserve formatting and meaning. If the content is already in English, return it lightly polished. ' +
            COMMON_RULES,
        },
        {
          role: 'user',
          content: `Translate the following content into English:\n\n${ctx}`,
        },
      ];

    case 'translate_zh':
      return [
        {
          role: 'system',
          content:
            'You are a translation assistant inside a note-taking app. Translate the provided content into natural, fluent Simplified Chinese (简体中文). Preserve formatting and meaning. If the content is already in Chinese, return it lightly polished. ' +
            COMMON_RULES,
        },
        {
          role: 'user',
          content: `将以下内容翻译为简体中文：\n\n${ctx}`,
        },
      ];

    case 'continue':
      return [
        {
          role: 'system',
          content:
            'You are a writing assistant inside a note-taking app. Continue writing from where the provided content ends, matching its tone, style, language, and formatting. Output ONLY the continuation (do not repeat the original text). ' +
            COMMON_RULES,
        },
        {
          role: 'user',
          content: `Continue writing from the end of the following content:\n\n${ctx}`,
        },
      ];

    case 'explain':
      return [
        {
          role: 'system',
          content:
            'You are a knowledgeable tutor inside a note-taking app. Explain the selected concept or term clearly and concisely, in the same language as the surrounding content. Use examples when helpful. ' +
            COMMON_RULES,
        },
        {
          role: 'user',
          content: `Explain the following concept or content:\n\n${ctx}`,
        },
      ];

    case 'extract_todos':
      return [
        {
          role: 'system',
          content:
            'You are an assistant inside a note-taking app that extracts actionable items. Read the provided content and output a Markdown task list (- [ ] ...) of every action item, decision-to-make, or follow-up. If none exist, output "- [ ] (未发现待办)". Use the same language as the source. ' +
            COMMON_RULES,
        },
        {
          role: 'user',
          content: `Extract all action items from the following content:\n\n${ctx}`,
        },
      ];

    case 'custom': {
      const instruction = (userPrompt ?? '').trim();
      return [
        {
          role: 'system',
          content:
            'You are a professional writing assistant integrated into a note-taking app. Follow the user instruction to write, improve, expand, or transform the provided note content. ' +
            COMMON_RULES,
        },
        {
          role: 'user',
          content: instruction
            ? `Instruction: ${instruction}\n\nCurrent note content:\n${ctx}`
            : `Help improve the following content:\n\n${ctx}`,
        },
      ];
    }

    default: {
      // Exhaustiveness guard — unknown ids fall back to a no-op user message.
      const _exhaustive: never = actionId;
      return [
        { role: 'system', content: COMMON_RULES },
        { role: 'user', content: `Unknown action: ${String(_exhaustive)}` },
      ];
    }
  }
}

/**
 * Case-insensitive substring / keyword filter over the palette. An empty query
 * returns all actions. Matching uses both label and keywords so users can type
 * either the localized label or an English keyword.
 */
export function filterActions(actions: AiAction[], query: string): AiAction[] {
  const q = (query ?? '').trim().toLowerCase();
  if (!q) return actions;
  return actions.filter((a) => {
    if (a.label.toLowerCase().includes(q)) return true;
    return a.keywords.some((k) => k.includes(q));
  });
}

/** Look up an action by id (returns undefined if not found). */
export function getActionById(actions: AiAction[], id: AiActionId): AiAction | undefined {
  return actions.find((a) => a.id === id);
}
