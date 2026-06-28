pub mod ask;
pub mod chat;
pub mod compress;
pub mod write;

// Re-export the main public API functions for backward compatibility
pub use ask::{ask_with_ai_with_context, normalize_tool_path};
pub use chat::{
    build_effective_question, chat_with_ai_with_context, finalize_chat_with_ai_answer,
    prepare_chat_for_ai, rollback_last_user_turn, PreparedChatContext,
};
pub use compress::compress_chat_history_with_context;
pub use write::write_with_ai_with_context;
