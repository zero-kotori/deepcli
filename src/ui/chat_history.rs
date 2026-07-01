use crate::runtime::AgentRuntime;
use crate::session::SessionMessage;
use anyhow::Result;

const TUI_HISTORY_MESSAGE_CHARS: usize = 4_000;

#[derive(Debug)]
pub(super) struct ChatLine {
    pub(super) role: String,
    pub(super) content: String,
}

pub(super) fn chat_lines_from_runtime(runtime: &AgentRuntime) -> Result<Vec<ChatLine>> {
    Ok(session_messages_to_chat_lines(runtime.session_messages()?))
}

pub(super) fn session_messages_to_chat_lines(messages: Vec<SessionMessage>) -> Vec<ChatLine> {
    messages
        .into_iter()
        .filter(|message| !message.content.trim().is_empty())
        .map(|message| ChatLine {
            role: match message.role.as_str() {
                "user" => "你".to_string(),
                "assistant" => "deepcli".to_string(),
                other => other.to_string(),
            },
            content: truncate_history_message(&message.content),
        })
        .collect()
}

fn truncate_history_message(content: &str) -> String {
    let char_count = content.chars().count();
    if char_count <= TUI_HISTORY_MESSAGE_CHARS {
        return content.to_string();
    }
    let mut truncated = content
        .chars()
        .take(TUI_HISTORY_MESSAGE_CHARS)
        .collect::<String>();
    truncated.push_str(&format!(
        "\n[deepcli truncated UI history: original_chars={char_count}]"
    ));
    truncated
}
