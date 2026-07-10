/**
 * AI quick action definitions and execution for the mobile command palette.
 *
 * Mirrors the Rust definitions in src/ai/actions.rs — each action has
 * a system prompt, user prompt template, and execution wrapper that
 * reuses the mobile app's existing chat() infrastructure.
 */

import { chat, ChatMessage, parseSSEStream } from '../api/client';

// ── Action type ──────────────────────────────────────────────

export type AiActionId =
  | 'summarize'
  | 'rewrite'
  | 'translate'
  | 'explain'
  | 'continueWriting'
  | 'extractTodos'
  | 'findRelatedNotes'
  | 'cleanUp';

export interface AiActionInfo {
  id: AiActionId;
  label: string;
  /** Ionicons icon name for the palette list. */
  icon: string;
  /** Description shown in search results. */
  description: string;
}

export interface AiActionRequest {
  action: AiActionId;
  text: string;
  targetLanguage?: string;
  tone?: string;
  noteId?: string;
}

export interface AiActionUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
}

export interface AiActionResult {
  result: string;
  usage: AiActionUsage;
  error?: string;
  get isSuccess(): boolean;
}

// ── Action metadata ─────────────────────────────────────────

const AI_ACTIONS: AiActionInfo[] = [
  {
    id: 'summarize',
    label: '总结',
    icon: 'document-text-outline',
    description: '将选中的文本或笔记精简为要点',
  },
  {
    id: 'rewrite',
    label: '改写',
    icon: 'create-outline',
    description: '按指定语气重写文本（正式/简洁/生动）',
  },
  {
    id: 'translate',
    label: '翻译',
    icon: 'language-outline',
    description: '翻译文本到目标语言',
  },
  {
    id: 'explain',
    label: '解释',
    icon: 'bulb-outline',
    description: '解释选中的概念或术语',
  },
  {
    id: 'continueWriting',
    label: '续写',
    icon: 'pencil-outline',
    description: '根据开头继续写作',
  },
  {
    id: 'extractTodos',
    label: '提取待办',
    icon: 'checkbox-outline',
    description: '从笔记中提取行动项目和待办事项',
  },
  {
    id: 'findRelatedNotes',
    label: '关联笔记',
    icon: 'link-outline',
    description: '找到与当前内容最相关的笔记',
  },
  {
    id: 'cleanUp',
    label: '整理',
    icon: 'sparkles-outline',
    description: '将凌乱、速记或语音转录的笔记整理成可读的结构化文本',
  },
];

/** Return all available AI quick actions (immutable copy). */
export function listAiActions(): AiActionInfo[] {
  return AI_ACTIONS.map(a => ({ ...a }));
}

/** Look up an action info by id. */
export function getAiActionInfo(id: string): AiActionInfo | undefined {
  return AI_ACTIONS.find(a => a.id === id);
}

// ── Prompt builders ──────────────────────────────────────────

function systemPrompt(action: AiActionId): string {
  switch (action) {
    case 'summarize':
      return 'You are a text summarization assistant. Your task is to distill the given text into concise, well-structured key points. Output only the summary, no extra commentary.\nRespond in the same language as the input text.';
    case 'rewrite':
      return 'You are a writing assistant. Rewrite the given text according to the specified tone. If no tone is specified, rewrite it in a clear, professional style. Preserve all factual information.\nRespond in the same language as the input text.';
    case 'translate':
      return 'You are a professional translator. Translate the given text to the specified target language. If no target language is specified, detect the source language and translate to English (or Chinese if the source is English). Output only the translation.';
    case 'explain':
      return 'You are a knowledgeable explainer. Explain the given concept, term, or passage in clear, accessible language. Provide context and examples where helpful. Respond in the same language as the input.';
    case 'continueWriting':
      return 'You are a creative writing assistant. Continue writing from the given text naturally, maintaining the same style, tone, and context. Output only the continuation without prefix phrases.';
    case 'extractTodos':
      return 'You are a task extraction assistant. Analyze the given text and extract all action items, tasks, to-dos, and follow-ups. Format the output as a bullet-point list with clear descriptions. If no tasks are found, state that explicitly.\nRespond in the same language as the input text.';
    case 'findRelatedNotes':
      return 'You are a knowledge base assistant. Analyze the given text and describe what topics, keywords, and concepts it covers. This description will be used for a search query to find related notes in the vault. Output a concise search description.';
    case 'cleanUp':
      return 'You are a note-formatting assistant. Your task is to clean up messy, rushed, or voice-transcribed notes into readable, well-structured text. Preserve all factual content and key information.\n- Fix typos and grammar where context makes the intent clear.\n- Organize run-on sentences into logical paragraphs.\n- Add bullet points or numbered lists where the content naturally has lists or enumerations.\n- Add headings (H2, H3) to break up long text thematically.\n- Remove repetitive or filler content.\n- Keep the original language and tone.\nOutput only the cleaned-up text, no extra commentary.';
  }
}

function userPrompt(action: AiActionId, request: AiActionRequest): string {
  switch (action) {
    case 'summarize':
      return `Please summarize the following text into key points:\n\n${request.text}`;
    case 'rewrite': {
      const tone = request.tone || 'professional';
      return `Please rewrite the following text with a ${tone} tone:\n\n${request.text}`;
    }
    case 'translate': {
      const lang = request.targetLanguage || 'English (or Chinese if the source is English)';
      return `Translate the following text to ${lang}:\n\n${request.text}`;
    }
    case 'explain':
      return `Please explain the following:\n\n${request.text}`;
    case 'continueWriting':
      return `Continue writing from the following text:\n\n${request.text}`;
    case 'extractTodos':
      return `Extract all action items, tasks, and to-dos from the following text:\n\n${request.text}`;
    case 'findRelatedNotes':
      return `Based on the following text, generate a search query to find related notes:\n\n${request.text}`;
    case 'cleanUp':
      return `Please clean up and reorganize the following messy note. Fix typos, improve structure, add headings and lists where appropriate. Preserve all content.\n\n${request.text}`;
  }
}

// ── Execution ───────────────────────────────────────────────

/** Validate the request before sending. Returns an error string or null. */
function validateRequest(request: AiActionRequest): string | null {
  if (!request.text.trim() && request.action !== 'findRelatedNotes') {
    return '输入文本不能为空。';
  }
  return null;
}

/**
 * Execute an AI quick action by calling the configured AI provider
 * with the appropriate system prompt. Collects the full streaming
 * response and returns it as a single result.
 */
export async function executeAiAction(
  request: AiActionRequest,
  signal?: AbortSignal,
): Promise<AiActionResult> {
  // Validate
  const validationError = validateRequest(request);
  if (validationError) {
    return {
      result: '',
      usage: { promptTokens: 0, completionTokens: 0, totalTokens: 0 },
      error: validationError,
      get isSuccess() { return false; },
    };
  }

  const system = systemPrompt(request.action);
  const prompt = userPrompt(request.action, request);

  const messages: ChatMessage[] = [
    { role: 'system', content: system },
    { role: 'user', content: prompt },
  ];

  try {
    const stream = await chat(messages, signal);
    let fullResult = '';
    await parseSSEStream(stream, (chunk) => {
      if (chunk.done) return;
      if (chunk.content) fullResult += chunk.content;
    }, { signal });

    return {
      result: fullResult.trim(),
      usage: {
        promptTokens: 0, // Token counts not available from mobile API
        completionTokens: 0,
        totalTokens: 0,
      },
      error: undefined,
      get isSuccess() { return true; },
    };
  } catch (e: any) {
    // AbortError from user cancellation
    if ((e as Error).name === 'AbortError') {
      return {
        result: '',
        usage: { promptTokens: 0, completionTokens: 0, totalTokens: 0 },
        error: undefined,
        get isSuccess() { return false; },
      };
    }
    return {
      result: '',
      usage: { promptTokens: 0, completionTokens: 0, totalTokens: 0 },
      error: `AI 操作执行失败：${e.message || '请重试'}`,
      get isSuccess() { return false; },
    };
  }
}
