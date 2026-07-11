pub mod ask;
pub mod auto_organize;
pub mod chat;
pub mod compress;
pub mod deep_research;
pub mod event_bus;
pub mod scheduled_research;
pub mod serendipity;
pub mod table;
pub mod tweet_import;
pub mod write;

// Re-export the main public API functions for backward compatibility
pub use ask::{ask_with_ai_with_context, normalize_tool_path, normalize_tool_path_with_canonical};
pub use auto_organize::{run_auto_organize, AutoOrganizeSummary, AutoOrganizer};
pub use chat::{
    build_effective_question, chat_with_ai_with_context, finalize_chat_with_ai_answer,
    prepare_chat_for_ai, rollback_last_user_turn, PreparedChatContext,
};
pub use compress::compress_chat_history_with_context;
pub use deep_research::{
    run_deep_research, DeepResearchEvent, DeepResearchTier, ResearchCitation, ResearchPlan,
    ResearchResult, ResearchSubQuestion, SearchRoundResult,
};
pub use event_bus::{publish_note_changed, Event, NoteAction, NoteChanged};
pub use scheduled_research::{
    run_all_due_subscriptions, run_single_subscription, SubscriptionRunResult,
};
pub use table::table_with_ai_with_context;
pub use write::{revert_write, write_with_ai_with_context, WriteBackup};

// Re-export serendipity public API (#1943)
pub use serendipity::{generate_serendipity, SerendipityItem, SerendipityResult};
