use chrono::Utc;

use crate::models::{AiSkill, AiWorkflowManual, ConversationTurn, NoteDocument, NoteMeta};

pub fn workflow_manual() -> AiWorkflowManual {
    AiWorkflowManual {
        title: "AI Knowledge Workflow Manual".to_string(),
        version: "v6".to_string(),
        summary: "The assistant should behave like a compact, deterministic knowledge-base agent: choose one action, prefer exact local inspection when the user provides a path, use the vault when recall is needed, and answer naturally.".to_string(),
        skills: vec![
            AiSkill {
                id: "tool_selection".to_string(),
                title: "Tool Selection".to_string(),
                purpose: "Choose one action before answering.".to_string(),
                steps: vec![
                    "Read the current user turn and recent conversation.".to_string(),
                    "Choose exactly one next step: direct answer, local vault retrieval, local path inspection, machine inspection, or save note.".to_string(),
                    "Prefer deterministic inspection tools when the user provides an exact local path.".to_string(),
                ],
                outputs: vec!["Structured tool plan".to_string()],
                guardrails: vec![
                    "Do not answer during the planning step.".to_string(),
                    "Do not invent search results before the tool executes.".to_string(),
                    "Do not run the same tool with the same arguments more than once in the same turn.".to_string(),
                ],
            },
            AiSkill {
                id: "knowledge_retrieval".to_string(),
                title: "Knowledge Retrieval".to_string(),
                purpose: "Recover what the user previously recorded in the local vault.".to_string(),
                steps: vec![
                    "First inspect candidate notes and choose the most relevant ones to read.".to_string(),
                    "Then read the provided tool result and local notes.".to_string(),
                    "Summarize what was actually done before, including commands, fixes, and outcomes.".to_string(),
                ],
                outputs: vec!["Natural-language answer".to_string(), "Relevant citations".to_string()],
                guardrails: vec![
                    "Do not claim a past action that is not present in the provided notes.".to_string(),
                ],
            },
            AiSkill {
                id: "capture_note".to_string(),
                title: "Capture Note".to_string(),
                purpose: "Turn user content into a normalized note for future retrieval.".to_string(),
                steps: vec![
                    "Use only facts that appear in the message and images.".to_string(),
                    "Normalize title, summary, tags, keywords, and body.".to_string(),
                ],
                outputs: vec!["Structured note draft".to_string()],
                guardrails: vec![
                    "Do not fabricate missing root cause or validation.".to_string(),
                    "Mark unknown fields as pending confirmation.".to_string(),
                ],
            },
            AiSkill {
                id: "conversation_compaction".to_string(),
                title: "Conversation Compaction".to_string(),
                purpose: "Compress earlier dialogue into a short working memory when context gets long.".to_string(),
                steps: vec![
                    "Preserve user identity, preferences, current goal, prior retrieval results, saved records, and unresolved tasks.".to_string(),
                    "Keep the summary compact and continuation-friendly.".to_string(),
                ],
                outputs: vec!["Compact memory summary".to_string()],
                guardrails: vec![
                    "Do not drop critical facts needed to continue the session.".to_string(),
                    "Do not include decorative prose.".to_string(),
                ],
            },
        ],
    }
}

pub fn ingest_system_prompt() -> String {
    format!(
        "You convert user material into a structured Markdown knowledge note.\n\
         Date: {}\n\
         {}\n\
         Rules:\n\
         - Use only information from the user and attached images.\n\
         - Do not invent missing causes, fixes, or validation.\n\
         - Return strict JSON only, with no markdown fence.",
        Utc::now().format("%Y-%m-%d"),
        render_manual_for_model()
    )
}

pub fn ingest_user_prompt(raw_input: &str) -> String {
    format!(
        "Return strict JSON in this shape:\n\
         {{\"title\":\"\",\"summary\":\"\",\"tags\":[],\"keywords\":[],\"platform\":\"\",\"board\":\"\",\"kernel\":\"\",\"status\":\"\",\"body\":\"\"}}\n\
         The body must follow this Markdown template:\n\
         {}\n\n\
         User input:\n{}",
        generic_note_template(),
        raw_input
    )
}

pub fn answer_system_prompt() -> String {
    format!(
        "You are a local knowledge assistant.\n\
         Date: {}\n\
         {}\n\
         Rules:\n\
         - Use retrieved local notes when they help answer the question.\n\
         - Cite only notes that were actually provided.\n\
         - Answer naturally in the user's language.\n\
         - Return strict JSON only, with no markdown fence.",
        Utc::now().format("%Y-%m-%d"),
        render_manual_for_model()
    )
}

pub fn answer_user_prompt(
    question: &str,
    docs: &[NoteDocument],
    history: &[ConversationTurn],
) -> String {
    format!(
        "Return strict JSON in this shape:\n\
         {{\"answer\":\"\",\"citations\":[{{\"noteId\":\"\",\"title\":\"\",\"path\":\"\",\"snippet\":\"\"}}],\"noteDraft\":null}}\n\n\
         User question:\n{}\n\n\
         Recent conversation:\n{}\n\n\
         Retrieved local notes:\n{}",
        question,
        render_history(history),
        render_notes(docs)
    )
}

pub fn general_chat_system_prompt() -> String {
    format!(
        "You are a general AI assistant embedded in a local knowledge app.\n\
         Date: {}\n\
         {}\n\
         Rules:\n\
         - No useful local notes are available for this turn.\n\
         - Answer directly and naturally in the user's language.\n\
         - Return strict JSON only, with no markdown fence.",
        Utc::now().format("%Y-%m-%d"),
        render_manual_for_model()
    )
}

pub fn general_chat_user_prompt(question: &str, history: &[ConversationTurn]) -> String {
    format!(
        "Return strict JSON in this shape:\n\
         {{\"answer\":\"\",\"citations\":[],\"noteDraft\":null}}\n\n\
         Recent conversation:\n{}\n\n\
         User message:\n{}",
        render_history(history),
        question
    )
}

pub fn record_system_prompt() -> String {
    format!(
        "You are the memory-capture agent inside a local knowledge app.\n\
         Date: {}\n\
         {}\n\
         Rules:\n\
         - The user wants this content stored in the knowledge base.\n\
         - Read the user message and images carefully.\n\
         - Produce a short natural reply and a structured note draft.\n\
         - Return strict JSON only, with no markdown fence.",
        Utc::now().format("%Y-%m-%d"),
        render_manual_for_model()
    )
}

pub fn record_user_prompt(raw_input: &str, docs: &[NoteDocument]) -> String {
    format!(
        "Return strict JSON in this exact shape:\n\
         {{\"reply\":\"\",\"noteDraft\":{{\"title\":\"\",\"summary\":\"\",\"tags\":[],\"keywords\":[],\"platform\":\"\",\"board\":\"\",\"kernel\":\"\",\"status\":\"\",\"source\":\"captured\",\"body\":\"\"}}}}\n\
         noteDraft.body must follow this Markdown template:\n\
         {}\n\
         Rules:\n\
         - Preserve commands, paths, versions, and ordered steps exactly when they appear.\n\
         - Use a general knowledge-note structure, not a bug-only template.\n\
         - Mark unknown facts as 待确认 instead of inventing them.\n\n\
         Current user message to record:\n{}\n\n\
         Similar local notes:\n{}",
        generic_note_template(),
        raw_input,
        render_notes(docs)
    )
}

pub fn compression_system_prompt() -> String {
    format!(
        "You are a conversation compactor inside a local AI knowledge assistant.\n\
         Date: {}\n\
         Rules:\n\
         - Compress earlier conversation into a concise working memory for future turns.\n\
         - Preserve user identity, preferences, important facts, retrieved note conclusions, saved commands, unresolved tasks, and current goals.\n\
         - Do not invent facts.\n\
         - Prefer compact bullet points.\n\
         - Return strict JSON only, with no markdown fence.",
        Utc::now().format("%Y-%m-%d")
    )
}

pub fn compression_user_prompt(existing_summary: &str, history: &[ConversationTurn]) -> String {
    format!(
        "Return strict JSON in this exact shape:\n\
         {{\"summary\":\"\"}}\n\n\
         Existing summary memory:\n{}\n\n\
         Conversation to compress:\n{}\n\n\
         Produce a compact memory summary in the user's language.",
        if existing_summary.trim().is_empty() {
            "(none)"
        } else {
            existing_summary
        },
        render_history(history)
    )
}

pub fn tool_call_system_prompt() -> String {
    format!(
        "You are the tool-selection stage of a local AI knowledge assistant.\n\
         Date: {}\n\
         {}\n\
         Your job is to choose exactly one next action for the current turn.\n\
         Prefer deterministic tools over broad retrieval when the user provides an exact local path.\n\
         Available tools:\n\
         - none: answer directly without the knowledge base.\n\
         - search_notes: search the local vault for relevant notes.\n\
         - list_notes: inspect recent notes to answer overview-style questions about the library.\n\
         - list_directory: inspect a local directory on the machine.\n\
         - read_file: read a local file on the machine.\n\
         - save_note: store the user's content as a normalized note draft.\n\
         You may be called repeatedly after prior tool executions, so use the tool history to decide the next step.\n\
         Return strict JSON only, with no markdown fence.",
        Utc::now().format("%Y-%m-%d"),
        render_manual_for_model()
    )
}

pub fn tool_call_user_prompt(
    question: &str,
    has_images: bool,
    history: &[ConversationTurn],
    prior_tool_results: &[String],
) -> String {
    format!(
        "Return strict JSON in this exact shape:\n\
         {{\"tool\":\"none|search_notes|list_notes|list_directory|read_file|save_note\",\"query\":\"\",\"path\":\"\",\"limit\":6,\"noteDraft\":null}}\n\
         Decision order:\n\
         1. If the user's primary intent is to store, remember, log, or add content to the knowledge base, use save_note.\n\
         2. If the user provides a concrete local path and asks to inspect, check, list, read, verify, or see what is under it, use list_directory for a directory or read_file for a file.\n\
         3. If the user asks what is in the library or what notes exist, use list_notes.\n\
         4. If the user asks about previous fixes, commands, similar cases, procedures, or prior recorded work, use search_notes.\n\
         5. Use none only for obvious greetings, thanks, identity questions, or turns answerable from recent conversation alone.\n\
         Rules:\n\
         - Choose exactly one next action.\n\
         - When the user asks about a specific local path, prefer path inspection tools over search_notes.\n\
         - Use save_note only when the user explicitly wants content stored; do not save just because the message contains commands or steps.\n\
         - Use list_directory to inspect local folders when the user asks about files, directories, projects, logs, or a specific path.\n\
         - Use read_file after you already know a concrete file path and need the file contents.\n\
         - Never select list_directory or read_file with an empty path.\n\
         - Avoid repeating the same tool with the same arguments if the prior tool results already answered the question.\n\
         - If prior tool results already contain save_note, do not call save_note again in the same turn. Choose none.\n\
         - If prior tool results already contain the same successful tool call with the same arguments, choose none.\n\
         - When tool is save_note, noteDraft must be a full object with shape:\n\
           {{\"title\":\"\",\"summary\":\"\",\"tags\":[],\"keywords\":[],\"platform\":\"\",\"board\":\"\",\"kernel\":\"\",\"status\":\"\",\"source\":\"captured\",\"body\":\"\"}}\n\
         - Use list_notes for questions like what is in the library, what notes exist, what has been recorded, or show me the vault contents.\n\
         - Use search_notes for previous fixes, commands, similar cases, technical operations, troubleshooting, procedures, and retrieval from past work.\n\
         - query should be filled only for search_notes.\n\
         - path should be filled only for list_directory or read_file.\n\
         - limit should be between 3 and 8.\n\
         - Do not answer the user yet.\n\n\
         Recent conversation:\n{}\n\n\
         Prior tool results:\n{}\n\n\
         User message:\n{}\n\
         Has images: {}",
        render_history(history),
        render_tool_results(prior_tool_results),
        question,
        if has_images { "yes" } else { "no" }
    )
}

pub fn tool_result_system_prompt() -> String {
    format!(
        "You are the final-response stage of a local AI knowledge assistant.\n\
         Date: {}\n\
         {}\n\
         You have already received the result of a tool execution.\n\
         Use that result to answer the user naturally.\n\
         Return strict JSON only, with no markdown fence.",
        Utc::now().format("%Y-%m-%d"),
        render_manual_for_model()
    )
}

pub fn note_selection_system_prompt() -> String {
    format!(
        "You are the note-selection stage of a local AI knowledge assistant.\n\
         Date: {}\n\
         {}\n\
         Your job is to choose which candidate notes should actually be read in full.\n\
         Return strict JSON only, with no markdown fence.",
        Utc::now().format("%Y-%m-%d"),
        render_manual_for_model()
    )
}

pub fn note_selection_user_prompt(
    question: &str,
    candidates: &[NoteMeta],
    history: &[ConversationTurn],
) -> String {
    format!(
        "Return strict JSON in this exact shape:\n\
         {{\"noteIds\":[\"\"]}}\n\
         Rules:\n\
         - Choose the 1 to 4 most relevant candidate notes to read in full.\n\
         - Prioritize notes whose title, summary, keywords, or path suggest they describe the user's previous fix, command, or operation.\n\
         - For broad technical questions, choose notes that are likely to contain concrete commands or procedures.\n\
         - A note that contains even a partial but concrete command is still relevant and should usually be selected.\n\
         - If none are relevant, return an empty array.\n\n\
         Recent conversation:\n{}\n\n\
         User question:\n{}\n\n\
         Candidate notes:\n{}",
        render_history(history),
        question,
        render_candidate_notes(candidates)
    )
}

pub fn tool_result_user_prompt(
    question: &str,
    tool_name: &str,
    tool_result: &str,
    docs: &[NoteDocument],
    history: &[ConversationTurn],
) -> String {
    format!(
        "Return strict JSON in this shape:\n\
         {{\"answer\":\"\",\"citations\":[{{\"noteId\":\"\",\"title\":\"\",\"path\":\"\",\"snippet\":\"\"}}],\"noteDraft\":null}}\n\
         Rules:\n\
         - tool_name tells you which tool already ran.\n\
         - tool_result is the factual result of that tool.\n\
         - If docs are provided, use them for citations.\n\
         - If search_notes or list_notes returned zero results, say clearly that the local knowledge base does not currently contain a relevant note, then provide your own general suggestion.\n\
         - If notes were found, prioritize those notes in the answer and make it obvious what came from local records.\n\
         - If a retrieved note contains a concrete command or step that partially answers a broad user question, surface that command first instead of ignoring it.\n\
         - If the tool result says a note was saved, answer like a normal assistant confirming what was captured.\n\
         - Do not ask the tool to run again in this stage.\n\n\
         Recent conversation:\n{}\n\n\
         User question:\n{}\n\n\
         tool_name:\n{}\n\n\
         tool_result:\n{}\n\n\
         docs_for_citation:\n{}",
        render_history(history),
        question,
        tool_name,
        tool_result,
        render_notes(docs)
    )
}

pub fn multi_tool_result_user_prompt(
    question: &str,
    tool_results: &[String],
    docs: &[NoteDocument],
    history: &[ConversationTurn],
) -> String {
    format!(
        "Return strict JSON in this shape:\n\
         {{\"answer\":\"\",\"citations\":[{{\"noteId\":\"\",\"title\":\"\",\"path\":\"\",\"snippet\":\"\"}}],\"noteDraft\":null}}\n\
         Rules:\n\
         - tool_results contains the factual outputs of the tools already executed in this turn.\n\
         - Use the tool results directly. Do not pretend another tool ran.\n\
         - If local notes were found, prioritize them and cite only provided notes.\n\
         - If the tool results contain a concrete command, path, or file content that answers the user, surface it directly.\n\
         - If the tool results show that nothing relevant was found locally, say so clearly before giving general advice.\n\n\
         Recent conversation:\n{}\n\n\
         User question:\n{}\n\n\
         tool_results:\n{}\n\n\
         docs_for_citation:\n{}",
        render_history(history),
        question,
        render_tool_results(tool_results),
        render_notes(docs)
    )
}

fn render_history(history: &[ConversationTurn]) -> String {
    if history.is_empty() {
        return "(none)".to_string();
    }

    history
        .iter()
        .map(|turn| format!("{}: {}", turn.role, turn.text))
        .collect::<Vec<_>>()
        .join("\n")
}

fn generic_note_template() -> &'static str {
    "## 摘要\n\
## 背景/上下文\n\
## 关键信息\n\
## 操作步骤/命令\n\
## 结果/结论\n\
## 待确认事项\n\
## 关键词"
}

fn render_notes(docs: &[NoteDocument]) -> String {
    if docs.is_empty() {
        return "(none)".to_string();
    }

    docs.iter()
        .map(|doc| {
            format!(
                "NOTE_ID: {}\nTITLE: {}\nPATH: {}\nTAGS: {}\nKEYWORDS: {}\nCONTENT:\n{}\n",
                doc.meta.id,
                doc.meta.title,
                doc.meta.path,
                doc.meta.tags.join(", "),
                doc.meta.keywords.join(", "),
                doc.body
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}

fn render_candidate_notes(candidates: &[NoteMeta]) -> String {
    if candidates.is_empty() {
        return "(none)".to_string();
    }

    candidates
        .iter()
        .map(|note| {
            format!(
                "NOTE_ID: {}\nTITLE: {}\nSUMMARY: {}\nKEYWORDS: {}\nPATH: {}\n",
                note.id,
                note.title,
                note.summary,
                note.keywords.join(", "),
                note.path
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n")
}

fn render_tool_results(tool_results: &[String]) -> String {
    if tool_results.is_empty() {
        return "(none)".to_string();
    }

    tool_results.join("\n\n---\n\n")
}

fn render_manual_for_model() -> String {
    let manual = workflow_manual();
    let mut text = format!(
        "<ai_workflow_manual title=\"{}\" version=\"{}\">\n{}\n",
        manual.title, manual.version, manual.summary
    );

    for skill in manual.skills {
        text.push_str(&format!(
            "\n<skill id=\"{}\">\nTitle: {}\nPurpose: {}\nSteps:\n- {}\nOutputs:\n- {}\nGuardrails:\n- {}\n</skill>\n",
            skill.id,
            skill.title,
            skill.purpose,
            skill.steps.join("\n- "),
            skill.outputs.join("\n- "),
            skill.guardrails.join("\n- "),
        ));
    }

    text.push_str("</ai_workflow_manual>");
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversationTurn, NoteDocument, NoteMeta};

    #[test]
    fn workflow_manual_contains_tool_selection() {
        let manual = workflow_manual();
        let ids = manual
            .skills
            .iter()
            .map(|skill| skill.id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"tool_selection"));
        assert!(ids.contains(&"capture_note"));
    }

    #[test]
    fn tool_call_prompt_lists_available_tools() {
        let prompt = tool_call_user_prompt("???????", false, &[], &[]);
        assert!(prompt.contains("search_notes"));
        assert!(prompt.contains("list_notes"));
        assert!(prompt.contains("save_note"));
        assert!(!prompt.contains("run_command"));
        assert!(prompt.contains("specific local path"));
        assert!(prompt.contains("\"source\":\"captured\""));
    }

    #[test]
    fn tool_result_prompt_mentions_tool_result() {
        let prompt = tool_result_user_prompt("???????", "list_notes", "3 notes", &[], &[]);
        assert!(prompt.contains("tool_result"));
        assert!(prompt.contains("\"answer\""));
    }

    #[test]
    fn multi_tool_result_prompt_mentions_results() {
        let prompt = multi_tool_result_user_prompt(
            "??????????",
            &["list_directory returned 2 items".to_string()],
            &[],
            &[],
        );
        assert!(prompt.contains("tool_results"));
        assert!(prompt.contains("list_directory"));
    }

    #[test]
    fn render_history_empty_returns_none() {
        assert_eq!(render_history(&[]), "(none)");
    }

    #[test]
    fn render_history_formats_turns() {
        let turns = vec![
            ConversationTurn {
                role: "user".to_string(),
                text: "hello".to_string(),
            },
            ConversationTurn {
                role: "assistant".to_string(),
                text: "hi there".to_string(),
            },
        ];
        let rendered = render_history(&turns);
        assert!(rendered.contains("user: hello"));
        assert!(rendered.contains("assistant: hi there"));
        assert!(!rendered.contains("(none)"));
    }

    #[test]
    fn render_notes_empty_returns_none() {
        assert_eq!(render_notes(&[]), "(none)");
    }

    #[test]
    fn render_notes_formats_documents() {
        let docs = vec![NoteDocument {
            meta: NoteMeta {
                id: "n1".to_string(),
                title: "Test".to_string(),
                path: "/vault/test.md".to_string(),
                tags: vec!["tag1".to_string()],
                keywords: vec!["kw1".to_string()],
                ..Default::default()
            },
            body: "body text".to_string(),
        }];
        let rendered = render_notes(&docs);
        assert!(rendered.contains("NOTE_ID: n1"));
        assert!(rendered.contains("TITLE: Test"));
        assert!(rendered.contains("PATH: /vault/test.md"));
        assert!(rendered.contains("TAGS: tag1"));
        assert!(rendered.contains("KEYWORDS: kw1"));
        assert!(rendered.contains("CONTENT:\nbody text"));
    }

    #[test]
    fn render_candidate_notes_empty_returns_none() {
        assert_eq!(render_candidate_notes(&[]), "(none)");
    }

    #[test]
    fn render_candidate_notes_formats_metadata() {
        let candidates = vec![NoteMeta {
            id: "n2".to_string(),
            title: "Candidate".to_string(),
            summary: "A summary".to_string(),
            keywords: vec!["kw".to_string()],
            path: "/vault/c2.md".to_string(),
            ..Default::default()
        }];
        let rendered = render_candidate_notes(&candidates);
        assert!(rendered.contains("NOTE_ID: n2"));
        assert!(rendered.contains("TITLE: Candidate"));
        assert!(rendered.contains("SUMMARY: A summary"));
        assert!(rendered.contains("KEYWORDS: kw"));
    }

    #[test]
    fn render_tool_results_joins_with_separator() {
        let results = vec!["result1".to_string(), "result2".to_string()];
        let rendered = render_tool_results(&results);
        assert!(rendered.contains("result1"));
        assert!(rendered.contains("result2"));
        assert!(rendered.contains("---"));
    }

    #[test]
    fn render_tool_results_empty_returns_none() {
        assert_eq!(render_tool_results(&[]), "(none)");
    }

    #[test]
    fn system_prompts_contain_date_and_manual() {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        assert!(ingest_system_prompt().contains(&today));
        assert!(answer_system_prompt().contains(&today));
        assert!(record_system_prompt().contains(&today));
        assert!(compression_system_prompt().contains(&today));

        assert!(ingest_system_prompt().contains("ai_workflow_manual"));
        assert!(answer_system_prompt().contains("ai_workflow_manual"));
        assert!(record_system_prompt().contains("ai_workflow_manual"));
    }

    #[test]
    fn compression_prompt_does_not_include_manual() {
        let prompt = compression_system_prompt();
        assert!(!prompt.contains("ai_workflow_manual"));
        assert!(prompt.contains("compactor"));
    }
}
