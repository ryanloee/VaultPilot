pub mod ask;
pub mod chat;
pub mod compress;

// Re-export the main public API functions for backward compatibility
pub use ask::{ask_with_ai_with_context, normalize_tool_path};
pub use chat::{build_effective_question, chat_with_ai_with_context};
pub use compress::compress_chat_history_with_context;
