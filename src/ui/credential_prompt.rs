use super::{handle_prompt_input_key, ChatLine, MessageBox, TuiState};
use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug)]
pub(super) struct CredentialPrompt {
    pub(super) provider: String,
    pub(super) force: bool,
    pub(super) input: MessageBox,
}

#[derive(Debug)]
pub(super) struct CredentialPromptSpec {
    pub(super) provider: String,
    pub(super) force: bool,
}

pub(super) fn handle_tui_credential_set_for_state(state: &mut TuiState, input: &str) -> bool {
    let Some(parsed) = parse_tui_credential_set(input) else {
        return false;
    };
    match parsed {
        Ok(spec) => {
            state.credential_prompt = Some(CredentialPrompt {
                provider: spec.provider.clone(),
                force: spec.force,
                input: MessageBox::new(),
            });
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: format!(
                    "请输入 `{}` 的 API key。输入内容会隐藏显示，Enter 保存，Esc 取消。",
                    spec.provider
                ),
            });
            state.last_event = "credential prompt opened".to_string();
        }
        Err(error) => state.chat.push(ChatLine {
            role: "error".to_string(),
            content: error.to_string(),
        }),
    }
    true
}

pub(super) fn handle_credential_prompt_key(key: KeyEvent, state: &mut TuiState) {
    match key.code {
        KeyCode::Enter => confirm_credential_prompt(state),
        _ => handle_prompt_input_key(
            state
                .credential_prompt
                .as_mut()
                .map(|prompt| &mut prompt.input),
            key,
        ),
    }
}

fn confirm_credential_prompt(state: &mut TuiState) {
    let Some(prompt) = state.credential_prompt.take() else {
        return;
    };
    let api_key = prompt.input.buffer().trim().to_string();
    if api_key.is_empty() {
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: "apiKey 不能为空。".to_string(),
        });
        state.last_event = "credential prompt rejected".to_string();
        return;
    }
    let Some(runtime) = state.runtime.as_mut() else {
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: "当前 runtime 不可用。".to_string(),
        });
        return;
    };
    match runtime.store_provider_api_key(&prompt.provider, api_key, prompt.force) {
        Ok(output) => {
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: output,
            });
            state.last_event = "credentials updated".to_string();
        }
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            });
            state.last_event = "credentials update failed".to_string();
        }
    }
}

pub(super) fn parse_tui_credential_set(input: &str) -> Option<Result<CredentialPromptSpec>> {
    let trimmed = input.trim();
    if !trimmed.starts_with("/credentials") {
        return None;
    }
    let parts = match shell_words::split(trimmed) {
        Ok(parts) => parts,
        Err(error) => return Some(Err(anyhow!("failed to parse credentials command: {error}"))),
    };
    if parts.first().map(String::as_str) != Some("/credentials")
        || parts.get(1).map(String::as_str) != Some("set")
    {
        return None;
    }
    let Some(provider) = parts.get(2).filter(|value| !value.trim().is_empty()) else {
        return Some(Err(anyhow!("missing provider name")));
    };
    let mut force = false;
    for arg in parts.iter().skip(3) {
        match arg.as_str() {
            "--force" => force = true,
            "--stdin" => {
                return Some(Err(anyhow!(
                    "TUI 已提供隐藏输入框，请去掉 --stdin 后重新执行"
                )));
            }
            other => {
                return Some(Err(anyhow!(
                    "unsupported /credentials set option `{other}`"
                )));
            }
        }
    }
    Some(Ok(CredentialPromptSpec {
        provider: provider.to_string(),
        force,
    }))
}

pub(super) fn credential_prompt_hidden_body(prompt: &CredentialPrompt) -> String {
    "*".repeat(prompt.input.buffer().chars().count())
}

pub(super) fn credential_prompt_hidden_cursor(prompt: &CredentialPrompt) -> usize {
    prompt.input.buffer()[..prompt.input.cursor()]
        .chars()
        .count()
}
