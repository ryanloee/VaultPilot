use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::client::{send_request_with_temperature, RequestUsage};
use super::transcription::transcribe_audio;
use crate::models::AppSettings;

// ─── Action type enum ─────────────────────────────────────────────────

/// Supported AI quick action types for the global command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiActionType {
    /// Summarize selected text or note into key points.
    Summarize,
    /// Rewrite text with a specified tone (formal, concise, vivid).
    Rewrite,
    /// Translate text to a target language.
    Translate,
    /// Explain a selected concept or term.
    Explain,
    /// Continue writing from the given text.
    ContinueWriting,
    /// Extract action items / to-dos from text.
    ExtractTodos,
    /// Find notes related to the given content.
    FindRelatedNotes,
    /// Clean up messy/quick-captured notes into readable structure.
    CleanUp,
    /// Generate a structured outline for a topic or based on existing notes.
    GenerateOutline,
    /// Composer: edit an existing note via natural-language instruction (#1569).
    EditNote,
    /// Generate a structured summary for a URL/link note.
    SummarizeUrl,
    /// Brainstorm creative ideas, alternatives, or solutions based on a topic.
    Brainstorm,
    /// Review a note and suggest structural/content improvements without modifying (#3102).
    ReviewNote,
    /// Synthesize a structured wiki article from related notes by tag/folder,
    /// with inline citations back to source notes (#3128).
    SynthesizeWiki,
    /// Workspace-wide Q&A: plan subqueries, retrieve block-level context from
    /// the entire vault, synthesize answer with inline [[Note#^block-id]]
    /// citations.  Uses block_id infrastructure from #2998 (#3188).
    WorkspaceQuery,
    /// Transcribe an audio file to text using the OpenAI Whisper API (#3256).
    /// The `text` field of the request is treated as the audio file path.
    TranscribeAudio,
    /// AI-powered context-aware note link suggestion: while a user is writing
    /// and editing a note, the AI analyzes the current note content and
    /// suggests relevant unlinked notes in the vault (#3271).
    SuggestLinks,
    /// Ad-hoc multi-note synthesis: synthesize a cross-note summary, identify
    /// shared themes, missing links, and conflicts from selected notes (#3270).
    SynthesizeNotes,
    /// Export note content to a structured file format (XLSX/DOCX/HTML/PPTX).
    /// The `instruction` field carries the target format (#3276).
    ExportDocument,
    /// Generate an interactive HTML/JS widget block that can be embedded in a note
    /// (ROI calculator, quiz, org chart, etc.). The AI produces self-contained
    /// HTML with inline JS/CSS, sandboxed for safe rendering (#3042).
    GenerateWidget,
}

impl AiActionType {
    /// Human-readable Chinese label for display in the command palette.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Summarize => "总结",
            Self::Rewrite => "改写",
            Self::Translate => "翻译",
            Self::Explain => "解释",
            Self::ContinueWriting => "续写",
            Self::ExtractTodos => "提取待办",
            Self::FindRelatedNotes => "关联笔记",
            Self::CleanUp => "整理",
            Self::GenerateOutline => "大纲生成",
            Self::EditNote => "编辑笔记",
            Self::SummarizeUrl => "链接摘要",
            Self::Brainstorm => "头脑风暴",
            Self::ReviewNote => "审阅笔记",
            Self::SynthesizeWiki => "综合维基",
            Self::WorkspaceQuery => "工作区问答",
            Self::TranscribeAudio => "音频转写",
            Self::SuggestLinks => "智能推荐链接",
            Self::SynthesizeNotes => "笔记综合",
            Self::ExportDocument => "导出文档",
            Self::GenerateWidget => "交互组件",
        }
    }

    /// English identifier for IPC/HTTP transport.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Summarize => "summarize",
            Self::Rewrite => "rewrite",
            Self::Translate => "translate",
            Self::Explain => "explain",
            Self::ContinueWriting => "continueWriting",
            Self::ExtractTodos => "extractTodos",
            Self::FindRelatedNotes => "findRelatedNotes",
            Self::CleanUp => "cleanUp",
            Self::GenerateOutline => "generateOutline",
            Self::EditNote => "editNote",
            Self::SummarizeUrl => "summarizeUrl",
            Self::Brainstorm => "brainstorm",
            Self::ReviewNote => "reviewNote",
            Self::SynthesizeWiki => "synthesizeWiki",
            Self::WorkspaceQuery => "workspaceQuery",
            Self::TranscribeAudio => "transcribeAudio",
            Self::SuggestLinks => "suggestLinks",
            Self::SynthesizeNotes => "synthesizeNotes",
            Self::ExportDocument => "exportDocument",
            Self::GenerateWidget => "generateWidget",
        }
    }

    /// Parse from an English identifier string.
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "summarize" => Some(Self::Summarize),
            "rewrite" => Some(Self::Rewrite),
            "translate" => Some(Self::Translate),
            "explain" => Some(Self::Explain),
            "continueWriting" | "continue_writing" => Some(Self::ContinueWriting),
            "extractTodos" | "extract_todos" => Some(Self::ExtractTodos),
            "findRelatedNotes" | "find_related_notes" => Some(Self::FindRelatedNotes),
            "cleanUp" | "clean_up" => Some(Self::CleanUp),
            "generateOutline" | "generate_outline" | "outline" => Some(Self::GenerateOutline),
            "editNote" | "edit_note" => Some(Self::EditNote),
            "summarizeUrl" | "summarize_url" => Some(Self::SummarizeUrl),
            "brainstorm" => Some(Self::Brainstorm),
            "reviewNote" | "review_note" | "review" => Some(Self::ReviewNote),
            "synthesizeWiki" | "synthesize_wiki" | "wiki" => Some(Self::SynthesizeWiki),
            "workspaceQuery" | "workspace_query" | "workspaceQa" | "workspace" => {
                Some(Self::WorkspaceQuery)
            }
            "transcribeAudio" | "transcribe_audio" | "transcribe" => Some(Self::TranscribeAudio),
            "suggestLinks" | "suggest_links" | "suggest" => Some(Self::SuggestLinks),
            "synthesizeNotes" | "synthesize_notes" | "synNotes" | "multiNoteSynthesis" => {
                Some(Self::SynthesizeNotes)
            }
            "exportDocument" | "export_document" | "export" => Some(Self::ExportDocument),
            "generateWidget" | "generate_widget" | "widget" => Some(Self::GenerateWidget),
            _ => None,
        }
    }

    /// Returns all available action types (for the command palette list).
    pub fn all() -> Vec<AiActionType> {
        vec![
            Self::Summarize,
            Self::Rewrite,
            Self::Translate,
            Self::Explain,
            Self::ContinueWriting,
            Self::ExtractTodos,
            Self::FindRelatedNotes,
            Self::CleanUp,
            Self::GenerateOutline,
            Self::EditNote,
            Self::SummarizeUrl,
            Self::Brainstorm,
            Self::ReviewNote,
            Self::SynthesizeWiki,
            Self::WorkspaceQuery,
            Self::TranscribeAudio,
            Self::SuggestLinks,
            Self::SynthesizeNotes,
            Self::ExportDocument,
            Self::GenerateWidget,
        ]
    }
}

// ─── Request / Response types ───────────────────────────────────────────

/// Parameters for executing an AI quick action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiActionRequest {
    /// The action type to perform.
    pub action: AiActionType,
    /// The text content to operate on (selected text, note body, etc.).
    #[serde(default)]
    pub text: String,
    /// Optional parameter: target language for translation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_language: Option<String>,
    /// Optional parameter: target tone for rewrite (formal, concise, vivid).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    /// Optional parameter: note ID for context (e.g., find_related_notes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    /// Composer: natural-language edit instruction for EditNote (#1569).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    /// Optional model override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Export format for ExportDocument action: "xlsx", "docx", "html", "pdf", "pptx" (#3276).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_format: Option<String>,
}

/// Result of an executed AI quick action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiActionResult {
    /// The resulting text after the action was applied.
    pub result: String,
    /// Token usage statistics.
    #[serde(default)]
    pub usage: RequestUsage,
    /// Error message if the action failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ─── Prompt builders ───────────────────────────────────────────────────

/// Build the system prompt for a given AI action type.
pub(crate) fn system_prompt(action: AiActionType) -> String {
    match action {
        AiActionType::Summarize => {
            "You are a text summarization assistant. Your task is to distill the \
             given text into concise, well-structured key points. \
             Output only the summary, no extra commentary.\n\
             Respond in the same language as the input text."
                .to_string()
        }
        AiActionType::Rewrite => {
            "You are a writing assistant. Rewrite the given text according to the \
             specified tone. If no tone is specified, rewrite it in a clear, \
             professional style. Preserve all factual information.\n\
             Respond in the same language as the input text."
                .to_string()
        }
        AiActionType::Translate => {
            "You are a professional translator. Translate the given text to the \
             specified target language. If no target language is specified, \
             detect the source language and translate to English (or Chinese if \
             the source is English). Output only the translation."
                .to_string()
        }
        AiActionType::Explain => {
            "You are a knowledgeable explainer. Explain the given concept, term, \
             or passage in clear, accessible language. Provide context and \
             examples where helpful. Respond in the same language as the input."
                .to_string()
        }
        AiActionType::ContinueWriting => {
            "You are a creative writing assistant. Continue writing from the \
             given text naturally, maintaining the same style, tone, and context. \
             Output only the continuation without prefix phrases."
                .to_string()
        }
        AiActionType::ExtractTodos => {
            "You are a task extraction assistant. Analyze the given text and \
             extract all action items, tasks, to-dos, and follow-ups. \
             Format the output as a bullet-point list with clear descriptions. \
             If no tasks are found, state that explicitly.\n\
             Respond in the same language as the input text."
                .to_string()
        }
        AiActionType::FindRelatedNotes => {
            "You are a knowledge base assistant. Analyze the given text and \
             describe what topics, keywords, and concepts it covers. \
             This description will be used for a search query to find related \
             notes in the vault. Output a concise search description."
                .to_string()
        }
        AiActionType::CleanUp => "You are a note-formatting assistant. Your task is to clean up \
             messy, rushed, or voice-transcribed notes into readable, \
             well-structured text. Preserve all factual content and key \
             information. \
             - Fix typos and grammar where context makes the intent clear. \
             - Organize run-on sentences into logical paragraphs. \
             - Add bullet points or numbered lists where the content \
               naturally has lists or enumerations. \
             - Add headings (H2, H3) to break up long text thematically. \
             - Remove repetitive or filler content. \
             - Keep the original language and tone. \
             Output only the cleaned-up text, no extra commentary."
            .to_string(),
        AiActionType::GenerateOutline => {
            "You are a knowledge-work assistant specializing in outline generation. \
             Your task is to generate a well-structured outline for the given topic \
             or content. The outline should use hierarchical numbering (e.g., 1, 1.1, \
             1.1.1) and be organized into logical sections with clear headings. \
             Include 3-5 main sections, each with 2-4 subsections. \
             - Base the outline on the provided text, expanding and structuring it \
               into a logical document framework. \
             - If the input is a topic rather than full text, generate a comprehensive \
               outline covering the key aspects of that topic. \
             - Add brief descriptions (one sentence) under each section heading \
               explaining what that section should cover. \
             - Use the same language as the input text. \
             Output only the outline in Markdown format, no extra commentary."
                .to_string()
        }
        AiActionType::EditNote => "You are a note editing assistant (Composer). Apply the user's \
             editing instruction to the given note text. Return the COMPLETE \
             edited note — not a diff, not a fragment, but the full text with \
             the requested changes applied. Preserve all content that the \
             instruction does not explicitly modify. Maintain the original \
             Markdown structure, headings, and formatting style.\n\
             Respond in the same language as the input text."
            .to_string(),
        AiActionType::SummarizeUrl => "You are a link summarization assistant. Your task is to analyze the content of a web page or article and generate a structured summary. Output a JSON object with these fields: title (the page title or concise description), key_points (an array of 3-5 key takeaways), summary (a 2-3 sentence overview), suggested_tags (2-4 relevant tags for categorization). Preserve factual accuracy. Do not hallucinate information not present in the source. Output only valid JSON, no extra commentary."
            .to_string(),
        AiActionType::Brainstorm => {
            "You are a creative brainstorming assistant. Your task is to generate \
             a diverse set of ideas, alternatives, perspectives, or solutions \
             based on the given topic or problem description. \
             - Think broadly and consider multiple angles. \
             - Be specific and actionable where possible. \
             - Organize ideas into clear categories or themes. \
             - Include both conventional and creative approaches. \
             Output only the brainstormed ideas, no extra commentary.\n\n\
             Respond in the same language as the input."
                .to_string()
        }
        AiActionType::ReviewNote => {
            "You are a knowledgeable document reviewer. Your task is to analyze the \
             given note and provide a structured review of its quality, structure, \
             and completeness. DO NOT rewrite or modify the text — only provide \
             observations and suggestions. \n\
             Output a structured review in the following format:\n\n\
             ## Structure Assessment\n\
             - Evaluate heading hierarchy, paragraph organization, and flow.\n\
             - Point out missing sections or logical gaps.\n\n\
             ## Content Completeness\n\
             - Identify topics that should be expanded.\n\
             - Note missing context, definitions, or examples.\n\n\
             ## Improvement Suggestions\n\
             - Specific, actionable suggestions (e.g., 'Section 2 could benefit from a code example').\n\
             - Suggested reorganization or new sections.\n\n\
             ## Clarity & Style\n\
             - Flag unclear expressions, jargon, or verbose passages.\n\
             - Suggest clearer alternatives.\n\n\
             Output the review in the same language as the input. \
             Be constructive and specific — avoid vague praise or criticism."
                .to_string()
        }
        AiActionType::SynthesizeWiki => {
            "You are a knowledge synthesis assistant. Your task is to synthesize a \
             structured wiki article from a collection of related notes (provided as \
             source material) centered on a given topic, tag, or folder. \
             - Organize the content into a coherent article with clear Markdown \
               headings (H2/H3) and logical sections. \
             - Integrate information from ALL provided source notes; do not invent \
               facts not present in the sources. \
             - Cite every claim back to its source using inline wikilinks in the \
               form [[note title]] or [[note title#heading]] so each statement is \
               traceable to a real note. Do NOT fabricate source references. \
             - If sources conflict, surface the discrepancy rather than silently \
               picking one. \
             - End with a short '## Sources' section listing the source note titles. \
             Output only the wiki article in Markdown, no extra commentary."
                .to_string()
        }
        AiActionType::WorkspaceQuery => {
            "You are a workspace-scale reasoning assistant. Your task is to answer \
             the user's question by reasoning across the entire vault. \
             - Plan sub-questions and retrieve relevant note blocks from the vault. \
             - Synthesize a comprehensive answer from the retrieved context. \
             - **Crucially**: cite every factual claim back to its source using \
               inline citations in the form [[Note Title#^block-id]], where \
               ^block-id is the exact block reference ID provided in the context \
               (e.g., `<!-- ^a1b2c3d4 -->` markers). \
             - If information is incomplete or sources conflict, note this \
               explicitly rather than inventing facts. \
             - Structure the answer with clear Markdown sections. End with a \
               '## Sources' section listing cited notes with their block references. \
             Output only the answer in Markdown with inline citations."
                .to_string()
        }
        // TranscribeAudio uses the Whisper API directly, not LLM chat completion.
        // The system prompt is unused — it's here only to satisfy the exhaustive match.
        AiActionType::TranscribeAudio => String::new(),
        AiActionType::SuggestLinks => {
            "You are a knowledge-graph assistant. Your task is to analyze the \
             current note content and a list of available notes in the vault. \
             Suggest 3-5 notes that are semantically related to the current \
             note but are NOT already linked in it (check the note's existing \
             wikilinks). Focus on notes that would create valuable connections. \
             Output format: a Markdown bullet list where each item is \
             `- [[Note Title]] — short reason (one sentence)`. \
             If no meaningful unlinked notes exist, output 'No relevant unlinked notes found.' \
             Output only the suggestion list, no extra commentary."
                .to_string()
        }
        AiActionType::SynthesizeNotes => {
            "You are a knowledge synthesis assistant specializing in multi-note \
             synthesis. Your task is to synthesize a structured report from \
             multiple selected notes. Output the following sections in Markdown:\n\
             ## Summary\n- A 2-3 paragraph cross-note summary.\n\n\
             ## Shared Themes\n- Bullet list of common themes across all notes.\n\n\
             ## Missing Links\n- Notes in the vault that should be linked but aren't \
               (if vault context not provided, skip this section).\n\n\
             ## Conflicts / Inconsistencies\n- Any contradictions between the \
               selected notes.\n\n\
             Cite specific source notes inline with [[Note Title]] wikilinks. \
             Do not invent facts not present in the sources. \
             Output only the structured report, no extra commentary."
                .to_string()
        }
        AiActionType::ExportDocument => {
            "You are a document formatting and export assistant. Your task is to \
             transform the given note content into a well-structured format suitable \
             for the target export type. \
             - For XLSX: restructure data into clean Markdown tables with headers. \
             - For DOCX: organize into clear headings, paragraphs, and lists. \
             - For HTML: produce clean, semantic Markdown that converts well to HTML. \
             - For PPTX: organize into slide-sized sections with H2 as slide titles. \
             Preserve all factual content. Enhance structure and readability. \
             Do not add new information. \
             Output only the restructured content in Markdown, no extra commentary."
                .to_string()
        }
        AiActionType::GenerateWidget => {
            "You are an interactive widget generator. Your task is to create a \
             self-contained interactive HTML widget based on the user's description. \
             Guidelines:\n\
             - Output ONLY the raw HTML code inside a ```html code block.\n\
             - Include all CSS and JavaScript inline within a single <!DOCTYPE html> document.\n\
             - NO external dependencies (no CDN links, no external fonts, no external libraries).\n\
             - Use vanilla JS only. For complex functionality you may inline a minimal library \
               via a <script> block if it is <= 5 KB after minification.\n\
             - The widget must be self-contained and work without a network connection.\n\
             - Follow accessibility best practices (aria labels, keyboard navigation, focus management).\n\
             - Use responsive design so it works on both desktop and mobile viewports.\n\
             - SECURITY: The output is automatically sanitized server-side (#3545). Scripts \
               containing fetch/XHR/WebSocket/localStorage/eval/Function() will be stripped. \
               Do NOT use inline event handlers (onclick, onload, etc.) — they will be removed. \
               Use addEventListener instead. The javascript: protocol is also blocked.\n\
             - Input fields must have appropriate types and validation.\n\
             - Provide clear visual feedback on user interactions (hover, focus, click, error states).\
             Output only the code block with the complete HTML document."
                .to_string()
        }
    }
}

/// Build the user prompt for a given AI action type.
pub(crate) fn user_prompt(action: AiActionType, request: &AiActionRequest) -> String {
    match action {
        AiActionType::Summarize => {
            format!(
                "Please summarize the following text into key points:\n\n{}",
                request.text
            )
        }
        AiActionType::Rewrite => {
            let tone = request.tone.as_deref().unwrap_or("professional");
            format!(
                "Please rewrite the following text with a {} tone:\n\n{}",
                tone, request.text
            )
        }
        AiActionType::Translate => {
            let language = request
                .target_language
                .as_deref()
                .unwrap_or("English (or Chinese if the source is English)");
            format!(
                "Translate the following text to {}:\n\n{}",
                language, request.text
            )
        }
        AiActionType::Explain => {
            format!("Please explain the following:\n\n{}", request.text)
        }
        AiActionType::ContinueWriting => {
            format!(
                "Continue writing from the following text:\n\n{}",
                request.text
            )
        }
        AiActionType::ExtractTodos => {
            format!(
                "Extract all action items, tasks, and to-dos from the following text:\n\n{}",
                request.text
            )
        }
        AiActionType::FindRelatedNotes => {
            format!(
                "Based on the following text, generate a search query to find related notes:\n\n{}",
                request.text
            )
        }
        AiActionType::CleanUp => {
            format!(
                "Please clean up and reorganize the following messy note. \
                 Fix typos, improve structure, add headings and lists where \
                 appropriate. Preserve all content.\n\n{}",
                request.text
            )
        }
        AiActionType::GenerateOutline => {
            format!(
                "Generate a structured outline based on the following topic or content:\n\n{}",
                request.text
            )
        }
        AiActionType::EditNote => {
            let instruction = request
                .instruction
                .as_deref()
                .unwrap_or("Improve this note.");
            format!(
                "Editing instruction: {}\n\nApply the instruction above to this note. \
                 Return the complete edited note:\n\n{}",
                instruction, request.text
            )
        }
        AiActionType::SummarizeUrl => {
            format!(
                "Analyze the following web page content and generate a structured summary.\n\n{}",
                request.text
            )
        }
        AiActionType::Brainstorm => {
            format!(
                "Please brainstorm creative ideas, solutions, or perspectives \
                 based on the following topic or problem:\n\n{}",
                request.text
            )
        }
        AiActionType::ReviewNote => {
            format!(
                "Please review the following note and provide a structured evaluation. \
                 Focus on structure, completeness, clarity, and actionable improvements:\n\n{}",
                request.text
            )
        }
        AiActionType::SynthesizeWiki => {
            let topic = request
                .instruction
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("the provided topic");
            format!(
                "Synthesize a structured wiki article about '{topic}' from the following \
                 related source notes. Integrate their content, cite each claim inline \
                 with [[note title]] wikilinks, and list the sources at the end:\n\n{}",
                request.text
            )
        }
        AiActionType::WorkspaceQuery => {
            let question = if request.text.trim().is_empty() {
                request
                    .instruction
                    .as_deref()
                    .unwrap_or("the provided question")
            } else {
                request.text.as_str()
            };
            format!(
                "Answer the following question by reasoning across all relevant notes \
                 in the vault. Break the question into sub-questions, search the vault \
                 for relevant context, and synthesize a comprehensive answer with \
                 inline citations such as [[Note Title#^block-id]] for every factual \
                 claim. Question:\n\n{}",
                question
            )
        }
        // TranscribeAudio uses the Whisper API directly, not LLM chat completion.
        // The user prompt is unused — it's here only to satisfy the exhaustive match.
        AiActionType::TranscribeAudio => String::new(),
        AiActionType::SuggestLinks => {
            format!(
                "Current note content:\n\n{}\n\n\
                 Available notes in the vault (title list):\n\n{}\n\n\
                 Based on the current note content, suggest 3-5 notes from the \
                 available notes list that are semantically related but NOT already \
                 linked as [[wikilinks]] in the current note. For each suggestion, \
                 include a one-sentence reason why they should be linked.",
                request.text,
                request
                    .instruction
                    .as_deref()
                    .unwrap_or("(no vault notes provided)")
            )
        }
        AiActionType::SynthesizeNotes => {
            format!(
                "Synthesize a multi-note report from the following selected notes.\n\
                 For each note, the format is:\n\
                 ## Note: <title>\n<content>\n---\n\n{}\n\n\
                 Include a cross-note summary, shared themes, missing links \
                 (if applicable), and any conflicts/inconsistencies. \
                 Cite specific source notes inline with [[Note Title]] wikilinks.",
                request.text
            )
        }
        AiActionType::ExportDocument => {
            let fmt = request
                .export_format
                .as_deref()
                .or(request.instruction.as_deref())
                .unwrap_or("html");
            format!(
                "Transform the following note content for export to {fmt} format. \
                 Restructure it for optimal presentation in the target format \
                 (clean tables for XLSX, well-organized headings/paragraphs for DOCX, \
                 semantic structure for HTML, slide-sized sections for PPTX). \
                 Preserve all factual content.\n\n{content}",
                fmt = fmt,
                content = request.text
            )
        }
        AiActionType::GenerateWidget => {
            format!(
                "Create an interactive HTML widget based on the following \
                 description. The widget should be self-contained with all \
                 CSS and JS inline. Output only the raw HTML code inside \
                 a ```html code block.\n\n{}",
                request.text
            )
        }
    }
}

// ─── Execution ─────────────────────────────────────────────────────────

/// Validate the action request synchronously. Returns an error result if
/// validation fails, or `None` if the request is valid.
pub(crate) fn validate_request(request: &AiActionRequest) -> Option<AiActionResult> {
    // WorkspaceQuery may supply its question via `instruction` instead of `text`,
    // so we only reject it when BOTH are empty (#3235).
    let has_text = !request.text.trim().is_empty();
    let has_instruction = request
        .instruction
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let requires_input = request.action != AiActionType::FindRelatedNotes;
    let instruction_optional = request.action == AiActionType::WorkspaceQuery;
    let empty = !has_text && (!instruction_optional || !has_instruction);
    if requires_input && empty {
        return Some(AiActionResult {
            result: String::new(),
            usage: RequestUsage::default(),
            error: Some("输入文本不能为空。".to_string()),
        });
    }
    None
}

/// Process a raw AI response into an [`AiActionResult`].
///
/// This is the pure (non-network) half of [`execute_ai_action`], extracted so it
/// can be unit-tested without a live provider. It implements the structured-JSON
/// contract: when the action is [`AiActionType::SummarizeUrl`], the model is asked
/// to return JSON but commonly wraps it in ```json fences or prepends prose, so we
/// run the existing `extract_json` helper. On failure to parse we surface an
/// `error` instead of returning non-parseable text (issue #3145). All other
/// actions keep returning the trimmed text verbatim.
pub(crate) fn process_action_result(
    action: AiActionType,
    raw_text: &str,
    usage: RequestUsage,
) -> AiActionResult {
    if action == AiActionType::SummarizeUrl {
        match crate::ai::parsing::extract_json(raw_text) {
            Ok(clean) => AiActionResult {
                result: clean,
                usage,
                error: None,
            },
            Err(e) => AiActionResult {
                result: String::new(),
                usage,
                error: Some(format!(
                    "AI 操作执行失败：无法解析结构化 JSON 结果：{}",
                    crate::sanitize_error(&e.to_string())
                )),
            },
        }
    } else if action == AiActionType::GenerateWidget {
        // Defense-in-depth XSS sanitization (#3545): strip network-exfiltration
        // scripts, event-handler attributes, dangerous tags, and javascript:
        // protocols before the HTML reaches any client renderer.
        let sanitized = crate::ai::html_sanitize::sanitize_widget_html(raw_text.trim());
        AiActionResult {
            result: sanitized,
            usage,
            error: None,
        }
    } else {
        AiActionResult {
            result: raw_text.trim().to_string(),
            usage,
            error: None,
        }
    }
}

/// Execute an AI quick action (non-streaming).
///
/// Returns the AI-generated result, or an error result if the AI call fails.
/// For [`TranscribeAudio`], the audio file path is taken from `request.text`
/// and transcription is performed via the OpenAI Whisper API.
#[instrument(skip(settings, request))]
pub async fn execute_ai_action(
    settings: &AppSettings,
    request: &AiActionRequest,
) -> AiActionResult {
    // Validate synchronously before making the AI call
    if let Some(error_result) = validate_request(request) {
        return error_result;
    }

    // TranscribeAudio uses the Whisper API directly, bypassing LLM chat completion.
    if request.action == AiActionType::TranscribeAudio {
        let provider = settings.effective_provider();
        let lang = request.target_language.as_deref();
        match transcribe_audio(&request.text, provider, lang).await {
            Ok(transcript) => AiActionResult {
                result: transcript,
                usage: RequestUsage::default(),
                error: None,
            },
            Err(e) => AiActionResult {
                result: String::new(),
                usage: RequestUsage::default(),
                error: Some(format!(
                    "音频转写失败：{}",
                    crate::sanitize_error(&e.to_string())
                )),
            },
        }
    } else {
        inner_execute_llm_action(settings, request).await
    }
}

/// Core path for LLM-based actions: build prompts, send chat completion, process result.
async fn inner_execute_llm_action(
    settings: &AppSettings,
    request: &AiActionRequest,
) -> AiActionResult {
    let system = system_prompt(request.action);
    let prompt = user_prompt(request.action, request);

    let mut action_settings = settings.clone();
    if let Some(ref model) = request.model {
        if !model.trim().is_empty() {
            action_settings.effective_provider_mut().model = model.clone();
        }
    }

    match send_request_with_temperature(&action_settings, &system, &prompt, &[], 0.3).await {
        Ok(response) => process_action_result(request.action, &response.text, response.usage),
        Err(e) => {
            let error_msg = format!("AI 操作执行失败：{}", crate::sanitize_error(&e.to_string()));
            AiActionResult {
                result: String::new(),
                usage: RequestUsage::default(),
                error: Some(error_msg),
            }
        }
    }
}

/// List all available AI actions with their metadata.
pub fn list_ai_actions() -> Vec<serde_json::Value> {
    AiActionType::all()
        .into_iter()
        .map(|action| {
            serde_json::json!({
                "id": action.id(),
                "label": action.label(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_type_roundtrip() {
        for action in AiActionType::all() {
            let id = action.id();
            let parsed = AiActionType::from_id(id);
            assert_eq!(parsed, Some(action), "roundtrip failed for {}", id);
        }
    }

    #[test]
    fn action_type_from_id_unknown() {
        assert_eq!(AiActionType::from_id("unknown_action"), None);
    }

    #[test]
    fn synthesize_alias_is_not_ambiguous() {
        // #3284 — from_id("synthesize") must NOT silently map to SynthesizeWiki
        assert_eq!(AiActionType::from_id("synthesize"), None);
        // But explicit aliases still work
        assert_eq!(
            AiActionType::from_id("synthesizeWiki"),
            Some(AiActionType::SynthesizeWiki)
        );
        assert_eq!(
            AiActionType::from_id("synthesizeNotes"),
            Some(AiActionType::SynthesizeNotes)
        );
    }

    #[test]
    fn action_type_from_id_alternative_names() {
        assert_eq!(
            AiActionType::from_id("continue_writing"),
            Some(AiActionType::ContinueWriting)
        );
        assert_eq!(
            AiActionType::from_id("extract_todos"),
            Some(AiActionType::ExtractTodos)
        );
        assert_eq!(
            AiActionType::from_id("find_related_notes"),
            Some(AiActionType::FindRelatedNotes)
        );
    }

    #[test]
    fn all_actions_have_labels() {
        for action in AiActionType::all() {
            assert!(!action.label().is_empty(), "label empty for {:?}", action);
            assert!(!action.id().is_empty(), "id empty for {:?}", action);
        }
    }

    #[test]
    fn empty_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::Summarize,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(result.is_some(), "empty text should fail validation");
        assert!(result.unwrap().error.unwrap().contains("空"));
    }

    #[test]
    fn whitespace_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::Summarize,
            text: "   ".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "whitespace-only text should fail validation"
        );
    }

    #[test]
    fn valid_text_passes_validation() {
        let request = AiActionRequest {
            action: AiActionType::Summarize,
            text: "Some real content".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        assert!(
            validate_request(&request).is_none(),
            "valid text should pass validation"
        );
    }

    #[test]
    fn find_related_notes_does_not_require_text() {
        let request = AiActionRequest {
            action: AiActionType::FindRelatedNotes,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        // find_related_notes is exempt from the text requirement
        assert!(validate_request(&request).is_none());
    }

    #[test]
    fn list_actions_returns_valid_list() {
        let actions = list_ai_actions();
        assert!(!actions.is_empty(), "should return at least one action");
        for action in &actions {
            assert!(
                action.get("id").and_then(|v| v.as_str()).is_some(),
                "action missing id"
            );
            assert!(
                action.get("label").and_then(|v| v.as_str()).is_some(),
                "action missing label"
            );
        }
    }

    #[test]
    fn system_prompt_not_empty() {
        for action in AiActionType::all() {
            // TranscribeAudio uses Whisper API directly, not LLM chat — its
            // system_prompt is intentionally empty (#3256).
            if action == AiActionType::TranscribeAudio {
                continue;
            }
            let prompt = system_prompt(action);
            assert!(!prompt.is_empty(), "system prompt empty for {:?}", action);
        }
    }

    #[test]
    fn user_prompt_contains_input_text() {
        let request = AiActionRequest {
            action: AiActionType::Summarize,
            text: "Hello world".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::Summarize, &request);
        assert!(prompt.contains("Hello world"));
    }

    #[test]
    fn user_prompt_with_tone() {
        let request = AiActionRequest {
            action: AiActionType::Rewrite,
            text: "Some text".to_string(),
            target_language: None,
            tone: Some("vivid".to_string()),
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::Rewrite, &request);
        assert!(prompt.contains("vivid"));
    }

    #[test]
    fn user_prompt_with_language() {
        let request = AiActionRequest {
            action: AiActionType::Translate,
            text: "Hello".to_string(),
            target_language: Some("Chinese".to_string()),
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::Translate, &request);
        assert!(prompt.contains("Chinese"));
    }

    #[test]
    fn cleanup_empty_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::CleanUp,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "empty text should fail validation for CleanUp"
        );
    }

    #[test]
    fn cleanup_user_prompt_contains_input() {
        let request = AiActionRequest {
            action: AiActionType::CleanUp,
            text: "meeting today discussed roadmap Q3 priorities".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::CleanUp, &request);
        assert!(prompt.contains("roadmap Q3"));
        assert!(prompt.contains("clean up"));
    }

    #[test]
    fn cleanup_system_prompt_includes_formatting_instructions() {
        let prompt = system_prompt(AiActionType::CleanUp);
        assert!(
            prompt.contains("bullets")
                || prompt.contains("headings")
                || prompt.contains("formatting")
                || prompt.contains("structure")
        );
        assert!(prompt.contains("Output only the cleaned-up text"));
    }

    // ── SummarizeUrl structured-JSON extraction (#3145) ──────────────

    #[test]
    fn summarize_url_strips_json_fence() {
        let fenced = "```json\n{\"title\":\"T\",\"key_points\":[\"a\"],\"summary\":\"s\",\"suggested_tags\":[\"x\"]}\n```";
        let result =
            process_action_result(AiActionType::SummarizeUrl, fenced, RequestUsage::default());
        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        assert!(
            result.result.starts_with('{') && result.result.ends_with('}'),
            "result should be clean JSON, got: {}",
            result.result
        );
        // Must be parseable by serde_json.
        assert!(
            serde_json::from_str::<serde_json::Value>(&result.result).is_ok(),
            "result must be valid JSON: {}",
            result.result
        );
    }

    #[test]
    fn summarize_url_strips_leading_prose() {
        let prose = "Here is the summary:\n{\"title\":\"T\",\"key_points\":[\"a\"],\"summary\":\"s\",\"suggested_tags\":[\"x\"]}";
        let result =
            process_action_result(AiActionType::SummarizeUrl, prose, RequestUsage::default());
        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        assert!(
            serde_json::from_str::<serde_json::Value>(&result.result).is_ok(),
            "result must be valid JSON: {}",
            result.result
        );
    }

    #[test]
    fn summarize_url_bare_json_passes_through() {
        let bare = "{\"title\":\"T\",\"key_points\":[],\"summary\":\"s\",\"suggested_tags\":[]}";
        let result =
            process_action_result(AiActionType::SummarizeUrl, bare, RequestUsage::default());
        assert!(
            result.error.is_none(),
            "unexpected error: {:?}",
            result.error
        );
        assert_eq!(result.result, bare);
    }

    #[test]
    fn summarize_url_non_json_surfaces_error() {
        let garbage = "I could not summarize that page.";
        let result =
            process_action_result(AiActionType::SummarizeUrl, garbage, RequestUsage::default());
        assert!(result.error.is_some(), "non-JSON should surface an error");
        assert!(
            result.result.is_empty(),
            "result must be empty on parse failure"
        );
    }

    #[test]
    fn non_summarize_url_actions_return_trimmed_text() {
        let fenced = "```json\n{\"foo\":1}\n```";
        let result =
            process_action_result(AiActionType::Summarize, fenced, RequestUsage::default());
        // Other actions must NOT strip fences — verbatim trimmed text.
        assert!(result.error.is_none());
        assert_eq!(result.result, fenced.trim());
    }

    // ── GenerateWidget HTML sanitization (#3545) ──────────────────────

    #[test]
    fn generate_widget_strips_xss_payloads() {
        let html = r#"<div onclick="alert(1)"><script>fetch('https://evil.com')</script></div>"#;
        let result =
            process_action_result(AiActionType::GenerateWidget, html, RequestUsage::default());
        assert!(result.error.is_none());
        assert!(
            !result.result.contains("fetch("),
            "fetch should be stripped"
        );
        assert!(
            !result.result.to_ascii_lowercase().contains("onclick"),
            "event handler should be stripped"
        );
        assert!(
            result.result.contains("<div"),
            "safe tag structure preserved"
        );
    }

    #[test]
    fn generate_widget_neutralizes_javascript_href() {
        let html = r#"<a href="javascript:alert(1)">click</a>"#;
        let result =
            process_action_result(AiActionType::GenerateWidget, html, RequestUsage::default());
        assert!(result.error.is_none());
        assert!(
            !result.result.contains("javascript:"),
            "javascript: protocol should be neutralized"
        );
    }

    #[test]
    fn generate_widget_preserves_safe_widget_html() {
        let html = r#"<button id="btn">Click</button>"#;
        let result =
            process_action_result(AiActionType::GenerateWidget, html, RequestUsage::default());
        assert!(result.error.is_none());
        assert!(result.result.contains("Click"), "safe content preserved");
        assert!(
            result.result.contains("id=\"btn\""),
            "safe attribute preserved"
        );
    }

    // ── GenerateOutline tests (#1830) ────────────────────────────────

    #[test]
    fn generate_outline_roundtrip() {
        assert_eq!(
            AiActionType::from_id("generateOutline"),
            Some(AiActionType::GenerateOutline)
        );
        assert_eq!(
            AiActionType::from_id("outline"),
            Some(AiActionType::GenerateOutline)
        );
        assert_eq!(
            AiActionType::from_id("generate_outline"),
            Some(AiActionType::GenerateOutline)
        );
        assert_eq!(AiActionType::GenerateOutline.id(), "generateOutline");
        assert_eq!(AiActionType::GenerateOutline.label(), "大纲生成");
    }

    #[test]
    fn generate_outline_in_all_actions() {
        let actions = AiActionType::all();
        assert!(
            actions.contains(&AiActionType::GenerateOutline),
            "GenerateOutline should be in all() list"
        );
    }

    #[test]
    fn generate_outline_system_prompt_not_empty() {
        let prompt = system_prompt(AiActionType::GenerateOutline);
        assert!(!prompt.is_empty());
        assert!(prompt.contains("outline"));
        assert!(prompt.contains("Markdown"));
    }

    #[test]
    fn generate_outline_user_prompt_contains_input() {
        let request = AiActionRequest {
            action: AiActionType::GenerateOutline,
            text: "项目管理最佳实践".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::GenerateOutline, &request);
        assert!(prompt.contains("项目管理最佳实践"));
        assert!(prompt.contains("outline"));
    }

    #[test]
    fn generate_outline_empty_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::GenerateOutline,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "empty text should fail validation for GenerateOutline"
        );
    }

    // ── Composer / EditNote tests (#1569) ────────────────────────────

    #[test]
    fn edit_note_label_and_id_are_consistent() {
        assert_eq!(AiActionType::EditNote.label(), "编辑笔记");
        assert_eq!(AiActionType::EditNote.id(), "editNote");
        assert_eq!(
            AiActionType::from_id("editNote"),
            Some(AiActionType::EditNote)
        );
        assert_eq!(
            AiActionType::from_id("edit_note"),
            Some(AiActionType::EditNote)
        );
    }

    #[test]
    fn edit_note_is_in_all_list() {
        let all = AiActionType::all();
        assert!(
            all.contains(&AiActionType::EditNote),
            "EditNote must be in the all() list"
        );
    }

    #[test]
    fn edit_note_system_prompt_instructs_complete_return() {
        let prompt = system_prompt(AiActionType::EditNote);
        assert!(
            prompt.contains("COMPLETE"),
            "must instruct to return complete note"
        );
        assert!(prompt.contains("Composer"), "must identify as Composer");
    }

    #[test]
    fn edit_note_user_prompt_includes_instruction_and_text() {
        let request = AiActionRequest {
            action: AiActionType::EditNote,
            text: "# My Note\n\nThis is the original content.".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: Some("Make it more formal".to_string()),
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::EditNote, &request);
        assert!(
            prompt.contains("Make it more formal"),
            "user prompt must include the instruction"
        );
        assert!(
            prompt.contains("original content"),
            "user prompt must include the note text"
        );
    }

    #[test]
    fn edit_note_user_prompt_defaults_instruction_when_none() {
        let request = AiActionRequest {
            action: AiActionType::EditNote,
            text: "Some text".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::EditNote, &request);
        assert!(
            !prompt.is_empty(),
            "should still produce a prompt with default instruction"
        );
    }

    #[test]
    fn edit_note_validates_non_empty_text() {
        let request = AiActionRequest {
            action: AiActionType::EditNote,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: Some("Edit this".to_string()),
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "empty text should fail validation for EditNote"
        );
    }

    // ── ReviewNote tests (#3102) ─────────────────────────────────────

    #[test]
    fn review_note_label_and_id_are_consistent() {
        assert_eq!(AiActionType::ReviewNote.label(), "审阅笔记");
        assert_eq!(AiActionType::ReviewNote.id(), "reviewNote");
        assert_eq!(
            AiActionType::from_id("reviewNote"),
            Some(AiActionType::ReviewNote)
        );
        assert_eq!(
            AiActionType::from_id("review_note"),
            Some(AiActionType::ReviewNote)
        );
        assert_eq!(
            AiActionType::from_id("review"),
            Some(AiActionType::ReviewNote)
        );
    }

    #[test]
    fn review_note_is_in_all_list() {
        let all = AiActionType::all();
        assert!(
            all.contains(&AiActionType::ReviewNote),
            "ReviewNote must be in the all() list"
        );
    }

    #[test]
    fn review_note_system_prompt_includes_review_sections() {
        let prompt = system_prompt(AiActionType::ReviewNote);
        assert!(!prompt.is_empty(), "system prompt must not be empty");
        // Must explicitly instruct NOT to modify the text
        assert!(
            prompt.contains("DO NOT rewrite") || prompt.contains("not modify"),
            "system prompt must instruct not to modify the note"
        );
        // Must include structured review sections
        assert!(
            prompt.contains("Structure Assessment"),
            "system prompt must request structure assessment"
        );
        assert!(
            prompt.contains("Content Completeness"),
            "system prompt must request content completeness review"
        );
        assert!(
            prompt.contains("Improvement Suggestions"),
            "system prompt must request improvement suggestions"
        );
        assert!(
            prompt.contains("Clarity"),
            "system prompt must include clarity review"
        );
    }

    #[test]
    fn review_note_user_prompt_contains_input_text() {
        let request = AiActionRequest {
            action: AiActionType::ReviewNote,
            text: "# 项目计划\n\n这是一份关于新产品的项目计划文档。".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::ReviewNote, &request);
        assert!(
            prompt.contains("项目计划"),
            "user prompt must include the input note text"
        );
        assert!(
            prompt.to_lowercase().contains("review"),
            "user prompt must reference review task"
        );
    }

    #[test]
    fn review_note_empty_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::ReviewNote,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "empty text should fail validation for ReviewNote"
        );
        assert!(
            result.unwrap().error.unwrap().contains("空"),
            "error message should mention empty input"
        );
    }

    #[test]
    fn review_note_whitespace_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::ReviewNote,
            text: "  \n\t  ".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "whitespace-only text should fail validation for ReviewNote"
        );
    }

    #[test]
    fn review_note_valid_text_passes_validation() {
        let request = AiActionRequest {
            action: AiActionType::ReviewNote,
            text: "# Note\n\nSome real content to review.".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        assert!(
            validate_request(&request).is_none(),
            "valid note text should pass validation for ReviewNote"
        );
    }

    #[test]
    fn review_note_roundtrip_via_action_type_roundtrip() {
        // The action_type_roundtrip test covers all actions in all().
        // Since ReviewNote is in all(), this explicitly confirms it round-trips.
        let id = AiActionType::ReviewNote.id();
        let parsed = AiActionType::from_id(id);
        assert_eq!(parsed, Some(AiActionType::ReviewNote));
    }

    // ── SynthesizeWiki tests (#3128) ──────────────────────────────────

    #[test]
    fn synthesize_wiki_label_and_id_are_consistent() {
        assert_eq!(AiActionType::SynthesizeWiki.label(), "综合维基");
        assert_eq!(AiActionType::SynthesizeWiki.id(), "synthesizeWiki");
        assert_eq!(
            AiActionType::from_id("synthesizeWiki"),
            Some(AiActionType::SynthesizeWiki)
        );
        assert_eq!(
            AiActionType::from_id("synthesize_wiki"),
            Some(AiActionType::SynthesizeWiki)
        );
        assert_eq!(
            AiActionType::from_id("wiki"),
            Some(AiActionType::SynthesizeWiki)
        );
    }

    #[test]
    fn synthesize_wiki_is_in_all_list() {
        let all = AiActionType::all();
        assert!(
            all.contains(&AiActionType::SynthesizeWiki),
            "SynthesizeWiki must be in the all() list"
        );
    }

    #[test]
    fn synthesize_wiki_system_prompt_instructs_citations() {
        let prompt = system_prompt(AiActionType::SynthesizeWiki);
        assert!(!prompt.is_empty());
        assert!(
            prompt.contains("wiki") || prompt.contains("Wiki"),
            "must describe wiki synthesis"
        );
        // Core constraint from #3128: citations must be real, not fabricated.
        assert!(
            prompt.contains("cite") || prompt.contains("Cite"),
            "must instruct to cite sources"
        );
        assert!(
            prompt.contains("[[") || prompt.contains("wikilink"),
            "must require wikilink-style citations back to source notes"
        );
        assert!(
            prompt.contains("Do NOT fabricate") || prompt.contains("fabricate"),
            "must forbid fabricating source references"
        );
    }

    #[test]
    fn synthesize_wiki_user_prompt_contains_sources_and_topic() {
        let request = AiActionRequest {
            action: AiActionType::SynthesizeWiki,
            text: "# Note A\ncontent...\n# Note B\nmore content...".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: Some("project planning".to_string()),
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::SynthesizeWiki, &request);
        assert!(
            prompt.contains("project planning"),
            "must include the topic"
        );
        assert!(prompt.contains("Note A"), "must include source note text");
        assert!(
            prompt.contains("[[note title]]"),
            "must instruct wikilink citations"
        );
    }

    #[test]
    fn synthesize_wiki_user_prompt_defaults_topic_when_no_instruction() {
        let request = AiActionRequest {
            action: AiActionType::SynthesizeWiki,
            text: "source material".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::SynthesizeWiki, &request);
        assert!(
            prompt.contains("the provided topic"),
            "must fall back to a default topic when instruction is absent"
        );
    }

    #[test]
    fn synthesize_wiki_empty_text_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::SynthesizeWiki,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "empty text should fail validation for SynthesizeWiki"
        );
    }

    // ── WorkspaceQuery tests (#3188) ──────────────────────────────────

    #[test]
    fn workspace_query_label_and_id() {
        assert_eq!(AiActionType::WorkspaceQuery.label(), "工作区问答");
        assert_eq!(AiActionType::WorkspaceQuery.id(), "workspaceQuery");
    }

    #[test]
    fn workspace_query_from_id() {
        assert_eq!(
            AiActionType::from_id("workspaceQuery"),
            Some(AiActionType::WorkspaceQuery)
        );
        assert_eq!(
            AiActionType::from_id("workspace_query"),
            Some(AiActionType::WorkspaceQuery)
        );
        assert_eq!(
            AiActionType::from_id("workspace"),
            Some(AiActionType::WorkspaceQuery)
        );
    }

    #[test]
    fn workspace_query_in_all_list() {
        let all = AiActionType::all();
        assert!(
            all.contains(&AiActionType::WorkspaceQuery),
            "WorkspaceQuery must be in the all() list"
        );
    }

    #[test]
    fn workspace_query_system_prompt_includes_block_id_citations() {
        let prompt = system_prompt(AiActionType::WorkspaceQuery);
        assert!(
            prompt.contains("^block-id"),
            "system prompt must instruct block-id citations"
        );
        assert!(
            prompt.contains("workspace-scale"),
            "must mention workspace-scale reasoning"
        );
    }

    #[test]
    fn workspace_query_user_prompt_includes_block_id() {
        let request = AiActionRequest {
            action: AiActionType::WorkspaceQuery,
            text: "What is the project timeline?".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::WorkspaceQuery, &request);
        assert!(
            prompt.contains("project timeline"),
            "must include the question text"
        );
        assert!(
            prompt.contains("[[Note Title#^block-id]]"),
            "must instruct inline citation format"
        );
    }

    #[test]
    fn workspace_query_user_prompt_falls_back_to_instruction() {
        let request = AiActionRequest {
            action: AiActionType::WorkspaceQuery,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: Some("timeline inquiry".to_string()),
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::WorkspaceQuery, &request);
        assert!(
            prompt.contains("timeline inquiry"),
            "must fall back to instruction when text is empty"
        );
        assert!(
            !prompt.is_empty(),
            "prompt must not be empty when instruction is provided"
        );
    }

    #[test]
    fn workspace_query_empty_all_returns_error() {
        let request = AiActionRequest {
            action: AiActionType::WorkspaceQuery,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "empty text should fail validation for WorkspaceQuery"
        );
    }

    // ── #3235 regression: instruction-only path must pass validation ──

    #[test]
    fn workspace_query_instruction_only_passes_validation() {
        // Regression for #3235: WorkspaceQuery with empty text but a non-empty
        // instruction must NOT be rejected by validate_request — otherwise the
        // user_prompt fallback to `instruction` is unreachable dead code.
        let request = AiActionRequest {
            action: AiActionType::WorkspaceQuery,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: Some("Summarize the Q3 roadmap across all project notes".to_string()),
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_none(),
            "WorkspaceQuery with a non-empty instruction must pass validation (#3235)"
        );
    }

    #[test]
    fn workspace_query_whitespace_instruction_still_rejected() {
        // Whitespace-only instruction must be treated as empty (#3235 edge case).
        let request = AiActionRequest {
            action: AiActionType::WorkspaceQuery,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: Some("   \n\t ".to_string()),
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "whitespace-only instruction must still fail validation for WorkspaceQuery"
        );
    }

    // ── TranscribeAudio tests (#3256) ─────────────────────────────────

    #[test]
    fn transcribe_audio_label_and_id() {
        assert_eq!(AiActionType::TranscribeAudio.label(), "音频转写");
        assert_eq!(AiActionType::TranscribeAudio.id(), "transcribeAudio");
    }

    #[test]
    fn transcribe_audio_from_id() {
        assert_eq!(
            AiActionType::from_id("transcribeAudio"),
            Some(AiActionType::TranscribeAudio)
        );
        assert_eq!(
            AiActionType::from_id("transcribe_audio"),
            Some(AiActionType::TranscribeAudio)
        );
        assert_eq!(
            AiActionType::from_id("transcribe"),
            Some(AiActionType::TranscribeAudio)
        );
    }

    #[test]
    fn transcribe_audio_in_all_list() {
        let all = AiActionType::all();
        assert!(
            all.contains(&AiActionType::TranscribeAudio),
            "TranscribeAudio must be in the all() list"
        );
    }

    #[test]
    fn transcribe_audio_empty_text_fails_validation() {
        let request = AiActionRequest {
            action: AiActionType::TranscribeAudio,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "empty text (no audio path) should fail validation"
        );
    }

    #[test]
    fn transcribe_audio_whitespace_text_fails_validation() {
        let request = AiActionRequest {
            action: AiActionType::TranscribeAudio,
            text: "   ".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&request);
        assert!(
            result.is_some(),
            "whitespace-only text should fail validation"
        );
    }

    #[test]
    fn transcribe_audio_with_path_passes_validation() {
        let request = AiActionRequest {
            action: AiActionType::TranscribeAudio,
            text: "/path/to/audio.mp3".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        assert!(validate_request(&request).is_none());
    }

    #[test]
    fn transcribe_audio_system_prompt_is_empty() {
        // TranscribeAudio bypasses LLM chat — its prompts are intentionally empty (#3256).
        let prompt = system_prompt(AiActionType::TranscribeAudio);
        assert_eq!(prompt, "");
    }

    #[test]
    fn transcribe_audio_user_prompt_is_empty() {
        let request = AiActionRequest {
            action: AiActionType::TranscribeAudio,
            text: "/tmp/audio.wav".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::TranscribeAudio, &request);
        assert_eq!(prompt, "");
    }

    // ── SuggestLinks tests (#3271) ─────────────────────────────────────

    #[test]
    fn suggest_links_label_and_id() {
        assert_eq!(AiActionType::SuggestLinks.label(), "智能推荐链接");
        assert_eq!(AiActionType::SuggestLinks.id(), "suggestLinks");
    }

    #[test]
    fn suggest_links_from_id_variants() {
        assert_eq!(
            AiActionType::from_id("suggestLinks"),
            Some(AiActionType::SuggestLinks)
        );
        assert_eq!(
            AiActionType::from_id("suggest_links"),
            Some(AiActionType::SuggestLinks)
        );
        assert_eq!(
            AiActionType::from_id("suggest"),
            Some(AiActionType::SuggestLinks)
        );
        assert_eq!(AiActionType::from_id("suggestLink"), None);
    }

    #[test]
    fn suggest_links_in_all_list() {
        let all = AiActionType::all();
        assert!(
            all.contains(&AiActionType::SuggestLinks),
            "SuggestLinks missing from all()"
        );
    }

    #[test]
    fn suggest_links_system_prompt_structure() {
        let prompt = system_prompt(AiActionType::SuggestLinks);
        assert!(!prompt.is_empty());
        assert!(prompt.contains("knowledge-graph"));
        assert!(prompt.contains("[[Note Title]]"));
        assert!(prompt.contains("unlinked"));
    }

    #[test]
    fn suggest_links_user_prompt_structure() {
        let req = AiActionRequest {
            action: AiActionType::SuggestLinks,
            text: "Some note content".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: Some("Note A\nNote B".to_string()),
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::SuggestLinks, &req);
        assert!(prompt.contains("Some note content"));
        assert!(prompt.contains("Note A"));
        assert!(prompt.contains("[[wikilinks]]"));
    }

    #[test]
    fn suggest_links_user_prompt_no_instruction() {
        let req = AiActionRequest {
            action: AiActionType::SuggestLinks,
            text: "Note content".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::SuggestLinks, &req);
        assert!(prompt.contains("Note content"));
        assert!(prompt.contains("no vault notes provided"));
    }

    #[test]
    fn suggest_links_empty_text_fails_validation() {
        let req = AiActionRequest {
            action: AiActionType::SuggestLinks,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: Some("some notes".to_string()),
            model: None,
            export_format: None,
        };
        let result = validate_request(&req);
        assert!(
            result.is_some(),
            "SuggestLinks with empty text should fail validation"
        );
    }

    #[test]
    fn suggest_links_process_action_result_trims() {
        let result = process_action_result(
            AiActionType::SuggestLinks,
            "  - [[Note A]] — related topic\n\n  ",
            RequestUsage::default(),
        );
        assert_eq!(result.result, "- [[Note A]] — related topic");
        assert!(result.error.is_none());
    }

    // ── SynthesizeNotes tests (#3270) ────────────────────────────────────

    #[test]
    fn synthesize_notes_label_and_id() {
        assert_eq!(AiActionType::SynthesizeNotes.label(), "笔记综合");
        assert_eq!(AiActionType::SynthesizeNotes.id(), "synthesizeNotes");
    }

    #[test]
    fn synthesize_notes_from_id_variants() {
        assert_eq!(
            AiActionType::from_id("synthesizeNotes"),
            Some(AiActionType::SynthesizeNotes)
        );
        assert_eq!(
            AiActionType::from_id("synthesize_notes"),
            Some(AiActionType::SynthesizeNotes)
        );
        assert_eq!(
            AiActionType::from_id("synNotes"),
            Some(AiActionType::SynthesizeNotes)
        );
    }

    #[test]
    fn synthesize_notes_in_all_list() {
        let all = AiActionType::all();
        assert!(
            all.contains(&AiActionType::SynthesizeNotes),
            "SynthesizeNotes missing from all()"
        );
    }

    #[test]
    fn synthesize_notes_system_prompt_structure() {
        let prompt = system_prompt(AiActionType::SynthesizeNotes);
        assert!(!prompt.is_empty());
        assert!(prompt.contains("Summary"));
        assert!(prompt.contains("Shared Themes"));
        assert!(prompt.contains("Missing Links"));
        assert!(prompt.contains("Conflicts"));
        assert!(prompt.contains("[[Note Title]]"));
    }

    #[test]
    fn synthesize_notes_user_prompt_structure() {
        let req = AiActionRequest {
            action: AiActionType::SynthesizeNotes,
            text: "## Note: Alpha\ncontent a\n---\n## Note: Beta\ncontent b\n---".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::SynthesizeNotes, &req);
        assert!(prompt.contains("Alpha"));
        assert!(prompt.contains("Beta"));
        assert!(prompt.contains("[[Note Title]]"));
    }

    #[test]
    fn synthesize_notes_empty_text_fails_validation() {
        let req = AiActionRequest {
            action: AiActionType::SynthesizeNotes,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&req);
        assert!(
            result.is_some(),
            "SynthesizeNotes with empty text should fail validation"
        );
    }

    #[test]
    fn synthesize_notes_process_action_result_trims() {
        let result = process_action_result(
            AiActionType::SynthesizeNotes,
            "  ## Summary\ncombined text\n  ",
            RequestUsage::default(),
        );
        assert_eq!(result.result, "## Summary\ncombined text");
        assert!(result.error.is_none());
    }

    // ─── GenerateWidget tests (#3042) ─────────────────────────────────

    #[test]
    fn generate_widget_roundtrip() {
        let id = AiActionType::GenerateWidget.id();
        let parsed = AiActionType::from_id(id);
        assert_eq!(parsed, Some(AiActionType::GenerateWidget));
    }

    #[test]
    fn generate_widget_from_id_aliases() {
        assert_eq!(
            AiActionType::from_id("generateWidget"),
            Some(AiActionType::GenerateWidget)
        );
        assert_eq!(
            AiActionType::from_id("generate_widget"),
            Some(AiActionType::GenerateWidget)
        );
        assert_eq!(
            AiActionType::from_id("widget"),
            Some(AiActionType::GenerateWidget)
        );
    }

    #[test]
    fn generate_widget_label_is_not_empty() {
        assert!(!AiActionType::GenerateWidget.label().is_empty());
    }

    #[test]
    fn generate_widget_in_all_list() {
        let all = AiActionType::all();
        assert!(
            all.contains(&AiActionType::GenerateWidget),
            "GenerateWidget missing from all()"
        );
    }

    #[test]
    fn generate_widget_system_prompt_has_expected_structure() {
        let prompt = system_prompt(AiActionType::GenerateWidget);
        assert!(!prompt.is_empty());
        assert!(
            prompt.contains("widget"),
            "system prompt should mention widget"
        );
        assert!(prompt.contains("HTML"), "system prompt should mention HTML");
    }

    #[test]
    fn generate_widget_user_prompt_contains_input_text() {
        let req = AiActionRequest {
            action: AiActionType::GenerateWidget,
            text: "Create an ROI calculator".to_string(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let prompt = user_prompt(AiActionType::GenerateWidget, &req);
        assert!(prompt.contains("ROI calculator"));
    }

    #[test]
    fn generate_widget_empty_text_fails_validation() {
        let req = AiActionRequest {
            action: AiActionType::GenerateWidget,
            text: String::new(),
            target_language: None,
            tone: None,
            note_id: None,
            instruction: None,
            model: None,
            export_format: None,
        };
        let result = validate_request(&req);
        assert!(
            result.is_some(),
            "GenerateWidget with empty text should fail validation"
        );
    }

    #[test]
    fn generate_widget_process_action_result_trims() {
        let result = process_action_result(
            AiActionType::GenerateWidget,
            "  ```html\n<div>Hello</div>\n```  ",
            RequestUsage::default(),
        );
        assert!(!result.result.is_empty());
        assert!(result.error.is_none());
    }
}
