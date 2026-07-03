use crate::config::ProviderRuntimeConfig;
use crate::privacy::redact_sensitive_text;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{env, time::Duration};
use tokio::time::sleep;

const DEFAULT_MAX_PROVIDER_ATTEMPTS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    Streaming,
    Reasoner,
    ToolCalling,
    JsonOutput,
    ContextCache,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<ProviderMessage>,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub json_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolSpec {
    #[serde(rename = "type")]
    pub spec_type: String,
    pub function: ToolFunctionSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolFunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallFunction {
    pub name: String,
    #[serde(
        serialize_with = "serialize_tool_arguments",
        deserialize_with = "deserialize_tool_arguments"
    )]
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub prompt_cache_hit_tokens: Option<u64>,
    #[serde(default)]
    pub prompt_cache_miss_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatResponse {
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StreamEvent {
    pub content_delta: Option<String>,
    pub reasoning_delta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_delta: Option<StreamToolCallDelta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_completed: Option<ToolCall>,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamToolCallDelta {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments_delta: Option<String>,
}

impl StreamEvent {
    fn text(content_delta: Option<String>, reasoning_delta: Option<String>, done: bool) -> Self {
        Self {
            content_delta,
            reasoning_delta,
            tool_call_delta: None,
            tool_call_completed: None,
            done,
        }
    }

    fn tool_delta(delta: StreamToolCallDelta) -> Self {
        Self {
            content_delta: None,
            reasoning_delta: None,
            tool_call_delta: Some(delta),
            tool_call_completed: None,
            done: false,
        }
    }

    fn tool_completed(call: ToolCall) -> Self {
        Self {
            content_delta: None,
            reasoning_delta: None,
            tool_call_delta: None,
            tool_call_completed: Some(call),
            done: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMetadata {
    pub name: String,
    pub provider_type: String,
    pub model: Option<String>,
    pub capabilities: Vec<String>,
}

pub type StreamEventCallback<'a> = &'a mut (dyn FnMut(StreamEvent) + Send);

#[async_trait]
pub trait ProviderClient: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
    async fn chat_with_stream_events(
        &self,
        request: ChatRequest,
        on_event: Option<StreamEventCallback<'_>>,
    ) -> Result<ChatResponse>;
    async fn stream(&self, request: ChatRequest) -> Result<Vec<StreamEvent>>;
    fn count_tokens(&self, messages: &[ProviderMessage]) -> usize;
    fn supports(&self, capability: ProviderCapability) -> bool;
    fn metadata(&self) -> ProviderMetadata;
}

pub fn create_provider(config: ProviderRuntimeConfig) -> Result<Box<dyn ProviderClient>> {
    match config.provider_type.as_str() {
        "deepseek" => Ok(Box::new(DeepSeekClient::new(config))),
        "kimi" => Ok(Box::new(KimiClient::new(config))),
        other => Err(anyhow!("provider type `{other}` is not implemented")),
    }
}

pub struct DeepSeekClient {
    config: ProviderRuntimeConfig,
    http: reqwest::Client,
}

impl DeepSeekClient {
    pub fn new(config: ProviderRuntimeConfig) -> Self {
        let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(30));
        let no_proxy = configured_no_proxy(&config.no_proxy);
        if let Some(proxy) = &config.http_proxy {
            if let Ok(proxy) = reqwest::Proxy::http(proxy) {
                builder = builder.proxy(proxy.no_proxy(no_proxy.clone()));
            }
        }
        if let Some(proxy) = &config.https_proxy {
            if let Ok(proxy) = reqwest::Proxy::https(proxy) {
                builder = builder.proxy(proxy.no_proxy(no_proxy.clone()));
            }
        }
        Self {
            config,
            http: builder.build().unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    fn endpoint(&self) -> &str {
        self.config
            .endpoint
            .as_deref()
            .unwrap_or("https://api.deepseek.com/chat/completions")
    }

    fn model(&self) -> &str {
        self.config.model.as_deref().unwrap_or("deepseek-chat")
    }

    fn api_key(&self) -> Result<&str> {
        self.config
            .api_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| anyhow!("DeepSeek apiKey is missing; configure .deepcli/credentials or DEEPSEEK_API_KEY"))
    }

    fn request_body(&self, request: &ChatRequest, stream: bool) -> Value {
        let messages = request
            .messages
            .iter()
            .cloned()
            .map(|mut message| {
                message.content = message
                    .content
                    .map(|content| redact_sensitive_text(&content));
                message.reasoning_content = message
                    .reasoning_content
                    .map(|content| redact_sensitive_text(&content));
                message
            })
            .collect::<Vec<_>>();
        let mut body = json!({
            "model": self.model(),
            "messages": messages,
            "stream": stream,
        });
        if !request.tools.is_empty() {
            body["tools"] = json!(request.tools);
            body["tool_choice"] = json!("auto");
        }
        if request.json_mode {
            body["response_format"] = json!({"type": "json_object"});
        }
        body
    }

    async fn chat_streaming_response(
        &self,
        request: ChatRequest,
        on_event: Option<StreamEventCallback<'_>>,
    ) -> Result<ChatResponse> {
        let body = self.request_body(&request, true);
        let max_attempts = provider_max_attempts();
        let mut last_error = None;
        for attempt in 0..max_attempts {
            match self
                .http
                .post(self.endpoint())
                .bearer_auth(self.api_key()?)
                .json(&body)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    return collect_streaming_chat_response(response, on_event).await;
                }
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    let message = format!(
                        "DeepSeek streaming chat failed with {status}: {}",
                        redact_secret(&text)
                    );
                    if is_retryable_status(status) && attempt + 1 < max_attempts {
                        last_error = Some(message);
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(anyhow!(message));
                }
                Err(error) if is_retryable_reqwest(&error) && attempt + 1 < max_attempts => {
                    last_error = Some(format!("DeepSeek streaming chat request failed: {error}"));
                    sleep(retry_delay(attempt)).await;
                }
                Err(error) => return Err(error).context("DeepSeek streaming chat request failed"),
            }
        }
        Err(anyhow!(
            "{}",
            last_error
                .unwrap_or_else(|| "DeepSeek streaming chat failed after retries".to_string())
        ))
    }
}

#[async_trait]
impl ProviderClient for DeepSeekClient {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.chat_with_stream_events(request, None).await
    }

    async fn chat_with_stream_events(
        &self,
        request: ChatRequest,
        on_event: Option<StreamEventCallback<'_>>,
    ) -> Result<ChatResponse> {
        if provider_streaming_chat_enabled() {
            return self.chat_streaming_response(request, on_event).await;
        }
        let body = self.request_body(&request, false);
        let max_attempts = provider_max_attempts();
        let mut last_error = None;
        for attempt in 0..max_attempts {
            match self
                .http
                .post(self.endpoint())
                .bearer_auth(self.api_key()?)
                .json(&body)
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    if status.is_success() {
                        if is_retryable_provider_body(&text) {
                            let message = "DeepSeek response body was empty".to_string();
                            if attempt + 1 < max_attempts {
                                last_error = Some(message);
                                sleep(retry_delay(attempt)).await;
                                continue;
                            }
                            return Err(anyhow!("{message} after {max_attempts} attempts"));
                        }
                        match normalize_chat_response(&text) {
                            Ok(response) => return Ok(response),
                            Err(error)
                                if is_retryable_provider_body(&text)
                                    && attempt + 1 < max_attempts =>
                            {
                                last_error =
                                    Some(format!("DeepSeek response could not be parsed: {error}"));
                                sleep(retry_delay(attempt)).await;
                                continue;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    let message = format!(
                        "DeepSeek request failed with {status}: {}",
                        redact_secret(&text)
                    );
                    if is_retryable_status(status) && attempt + 1 < max_attempts {
                        last_error = Some(message);
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(anyhow!(message));
                }
                Err(error) if is_retryable_reqwest(&error) && attempt + 1 < max_attempts => {
                    last_error = Some(format!("DeepSeek request failed: {error}"));
                    sleep(retry_delay(attempt)).await;
                }
                Err(error) => return Err(error).context("DeepSeek request failed"),
            }
        }
        Err(anyhow!(
            "{}",
            last_error.unwrap_or_else(|| "DeepSeek request failed after retries".to_string())
        ))
    }

    async fn stream(&self, request: ChatRequest) -> Result<Vec<StreamEvent>> {
        let body = self.request_body(&request, true);
        let max_attempts = provider_max_attempts();
        let mut response = None;
        let mut last_error = None;
        for attempt in 0..max_attempts {
            match self
                .http
                .post(self.endpoint())
                .bearer_auth(self.api_key()?)
                .json(&body)
                .send()
                .await
            {
                Ok(candidate) if candidate.status().is_success() => {
                    response = Some(candidate);
                    break;
                }
                Ok(candidate) => {
                    let status = candidate.status();
                    let text = candidate.text().await.unwrap_or_default();
                    let message = format!(
                        "DeepSeek streaming failed with {status}: {}",
                        redact_secret(&text)
                    );
                    if is_retryable_status(status) && attempt + 1 < max_attempts {
                        last_error = Some(message);
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(anyhow!(message));
                }
                Err(error) if is_retryable_reqwest(&error) && attempt + 1 < max_attempts => {
                    last_error = Some(format!("DeepSeek streaming request failed: {error}"));
                    sleep(retry_delay(attempt)).await;
                }
                Err(error) => return Err(error).context("DeepSeek streaming request failed"),
            }
        }

        let response = response.ok_or_else(|| {
            anyhow!(
                "{}",
                last_error.unwrap_or_else(
                    || "DeepSeek streaming request failed after retries".to_string()
                )
            )
        })?;

        let mut events = Vec::new();
        let mut buffer = String::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read provider stream")?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(index) = buffer.find('\n') {
                let line = buffer[..index].trim().to_string();
                buffer = buffer[index + 1..].to_string();
                if let Some(event) = parse_sse_line(&line)? {
                    let done = event.done;
                    events.push(event);
                    if done {
                        return Ok(events);
                    }
                }
            }
        }
        Ok(events)
    }

    fn supports(&self, capability: ProviderCapability) -> bool {
        let expected = capability_name(capability);
        self.config
            .capabilities
            .iter()
            .any(|value| value == expected)
    }

    fn count_tokens(&self, messages: &[ProviderMessage]) -> usize {
        estimate_tokens(messages)
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: self.config.name.clone(),
            provider_type: self.config.provider_type.clone(),
            model: self.config.model.clone(),
            capabilities: self.config.capabilities.clone(),
        }
    }
}

pub struct KimiClient {
    config: ProviderRuntimeConfig,
    http: reqwest::Client,
}

impl KimiClient {
    pub fn new(config: ProviderRuntimeConfig) -> Self {
        let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(30));
        let no_proxy = configured_no_proxy(&config.no_proxy);
        if let Some(proxy) = &config.http_proxy {
            if let Ok(proxy) = reqwest::Proxy::http(proxy) {
                builder = builder.proxy(proxy.no_proxy(no_proxy.clone()));
            }
        }
        if let Some(proxy) = &config.https_proxy {
            if let Ok(proxy) = reqwest::Proxy::https(proxy) {
                builder = builder.proxy(proxy.no_proxy(no_proxy.clone()));
            }
        }
        Self {
            config,
            http: builder.build().unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    fn endpoint(&self) -> &str {
        self.config
            .endpoint
            .as_deref()
            .unwrap_or("https://api.kimi.com/coding/v1/messages")
    }

    fn model(&self) -> &str {
        self.config.model.as_deref().unwrap_or("kimi-for-coding")
    }

    fn api_key(&self) -> Result<&str> {
        self.config
            .api_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                anyhow!("Kimi apiKey is missing; configure .deepcli/credentials or KIMI_API_KEY")
            })
    }

    async fn chat_anthropic_streaming_response(
        &self,
        request: ChatRequest,
        on_event: Option<StreamEventCallback<'_>>,
    ) -> Result<ChatResponse> {
        let body = kimi_anthropic_request_body(self.model(), &request, true);
        let max_attempts = provider_max_attempts();
        let mut last_error = None;
        for attempt in 0..max_attempts {
            match self
                .http
                .post(self.endpoint())
                .bearer_auth(self.api_key()?)
                .header("x-api-key", self.api_key()?)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    return collect_kimi_anthropic_streaming_chat_response(response, on_event)
                        .await;
                }
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    let message = format!(
                        "Kimi streaming chat failed with {status}: {}",
                        redact_secret(&text)
                    );
                    if is_retryable_status(status) && attempt + 1 < max_attempts {
                        last_error = Some(message);
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(anyhow!(message));
                }
                Err(error) if is_retryable_reqwest(&error) && attempt + 1 < max_attempts => {
                    last_error = Some(format!("Kimi streaming chat request failed: {error}"));
                    sleep(retry_delay(attempt)).await;
                }
                Err(error) => return Err(error).context("Kimi streaming chat request failed"),
            }
        }
        Err(anyhow!(
            "{}",
            last_error.unwrap_or_else(|| "Kimi streaming chat failed after retries".to_string())
        ))
    }

    async fn chat_anthropic_response(
        &self,
        request: ChatRequest,
        on_event: Option<StreamEventCallback<'_>>,
    ) -> Result<ChatResponse> {
        if provider_streaming_chat_enabled() {
            return self
                .chat_anthropic_streaming_response(request, on_event)
                .await;
        }

        let body = kimi_anthropic_request_body(self.model(), &request, false);
        let max_attempts = provider_max_attempts();
        let mut last_error = None;
        for attempt in 0..max_attempts {
            match self
                .http
                .post(self.endpoint())
                .bearer_auth(self.api_key()?)
                .header("x-api-key", self.api_key()?)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.unwrap_or_default();
                    if status.is_success() {
                        return normalize_kimi_anthropic_response(&text);
                    }
                    let message = format!(
                        "Kimi request failed with {status}: {}",
                        redact_secret(&text)
                    );
                    if is_retryable_status(status) && attempt + 1 < max_attempts {
                        last_error = Some(message);
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(anyhow!(message));
                }
                Err(error) if is_retryable_reqwest(&error) && attempt + 1 < max_attempts => {
                    last_error = Some(format!("Kimi request failed: {error}"));
                    sleep(retry_delay(attempt)).await;
                }
                Err(error) => return Err(error).context("Kimi request failed"),
            }
        }
        Err(anyhow!(
            "{}",
            last_error.unwrap_or_else(|| "Kimi request failed after retries".to_string())
        ))
    }
}

#[async_trait]
impl ProviderClient for KimiClient {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.chat_with_stream_events(request, None).await
    }

    async fn chat_with_stream_events(
        &self,
        request: ChatRequest,
        on_event: Option<StreamEventCallback<'_>>,
    ) -> Result<ChatResponse> {
        self.chat_anthropic_response(request, on_event).await
    }

    async fn stream(&self, request: ChatRequest) -> Result<Vec<StreamEvent>> {
        let body = kimi_anthropic_request_body(self.model(), &request, true);
        let max_attempts = provider_max_attempts();
        let mut response = None;
        let mut last_error = None;
        for attempt in 0..max_attempts {
            match self
                .http
                .post(self.endpoint())
                .bearer_auth(self.api_key()?)
                .header("x-api-key", self.api_key()?)
                .header("anthropic-version", "2023-06-01")
                .json(&body)
                .send()
                .await
            {
                Ok(candidate) if candidate.status().is_success() => {
                    response = Some(candidate);
                    break;
                }
                Ok(candidate) => {
                    let status = candidate.status();
                    let text = candidate.text().await.unwrap_or_default();
                    let message = format!(
                        "Kimi streaming failed with {status}: {}",
                        redact_secret(&text)
                    );
                    if is_retryable_status(status) && attempt + 1 < max_attempts {
                        last_error = Some(message);
                        sleep(retry_delay(attempt)).await;
                        continue;
                    }
                    return Err(anyhow!(message));
                }
                Err(error) if is_retryable_reqwest(&error) && attempt + 1 < max_attempts => {
                    last_error = Some(format!("Kimi streaming request failed: {error}"));
                    sleep(retry_delay(attempt)).await;
                }
                Err(error) => return Err(error).context("Kimi streaming request failed"),
            }
        }

        let response = response.ok_or_else(|| {
            anyhow!(
                "{}",
                last_error
                    .unwrap_or_else(|| "Kimi streaming request failed after retries".to_string())
            )
        })?;
        collect_kimi_anthropic_stream_events(response).await
    }

    fn supports(&self, capability: ProviderCapability) -> bool {
        let expected = capability_name(capability);
        self.config
            .capabilities
            .iter()
            .any(|value| value == expected)
    }

    fn count_tokens(&self, messages: &[ProviderMessage]) -> usize {
        estimate_tokens(messages)
    }

    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata {
            name: self.config.name.clone(),
            provider_type: self.config.provider_type.clone(),
            model: self.config.model.clone(),
            capabilities: self.config.capabilities.clone(),
        }
    }
}

fn kimi_anthropic_request_body(model: &str, request: &ChatRequest, stream: bool) -> Value {
    let mut system_parts = Vec::new();
    let mut messages = Vec::new();
    for message in &request.messages {
        match message.role.as_str() {
            "system" => {
                if let Some(content) = &message.content {
                    system_parts.push(redact_sensitive_text(content));
                }
            }
            "tool" => {
                let content = message.content.clone().unwrap_or_default();
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                        "content": redact_sensitive_text(&content)
                    }]
                }));
            }
            "assistant" => {
                let mut content_blocks = Vec::new();
                if let Some(content) = &message.content {
                    if !content.trim().is_empty() {
                        content_blocks.push(json!({
                            "type": "text",
                            "text": redact_sensitive_text(content)
                        }));
                    }
                }
                if let Some(tool_calls) = &message.tool_calls {
                    for call in tool_calls {
                        content_blocks.push(json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.function.name,
                            "input": call.function.arguments
                        }));
                    }
                }
                if content_blocks.is_empty() {
                    content_blocks.push(json!({"type": "text", "text": ""}));
                }
                messages.push(json!({
                    "role": "assistant",
                    "content": content_blocks
                }));
            }
            _ => {
                let content = message.content.clone().unwrap_or_default();
                messages.push(json!({
                    "role": "user",
                    "content": redact_sensitive_text(&content)
                }));
            }
        }
    }

    let mut body = json!({
        "model": model,
        "max_tokens": provider_max_output_tokens(),
        "messages": messages,
    });
    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }
    if !request.tools.is_empty() {
        body["tools"] = json!(request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.function.name,
                    "description": tool.function.description,
                    "input_schema": tool.function.parameters
                })
            })
            .collect::<Vec<_>>());
    }
    if stream {
        body["stream"] = json!(true);
    }
    body
}

fn normalize_kimi_anthropic_response(raw: &str) -> Result<ChatResponse> {
    let value: Value =
        serde_json::from_str(raw).context("failed to parse Kimi Anthropic response")?;
    if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
        return Err(anyhow!("Kimi request failed: {}", redact_secret(error)));
    }

    let mut content = String::new();
    let mut reasoning_content = String::new();
    let mut tool_calls = Vec::new();
    if let Some(blocks) = value.get("content").and_then(Value::as_array) {
        for (index, block) in blocks.iter().enumerate() {
            match block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "text" => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        content.push_str(text);
                    }
                }
                "thinking" => {
                    if let Some(text) = block.get("thinking").and_then(Value::as_str) {
                        reasoning_content.push_str(text);
                    }
                }
                "tool_use" => {
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("Kimi tool_use block missing name"))?;
                    let input = block
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| Value::Object(Default::default()));
                    tool_calls.push(ToolCall {
                        id: block
                            .get("id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("toolu_{index}")),
                        call_type: "function".to_string(),
                        function: ToolCallFunction {
                            name: name.to_string(),
                            arguments: input,
                        },
                    });
                }
                _ => {}
            }
        }
    }

    let usage = value.get("usage").cloned().unwrap_or(Value::Null);
    Ok(ChatResponse {
        content: (!content.is_empty()).then_some(content),
        reasoning_content: (!reasoning_content.is_empty()).then_some(reasoning_content),
        tool_calls,
        usage: kimi_usage_from_value(&usage),
    })
}

fn kimi_usage_from_value(usage: &Value) -> Usage {
    let prompt_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("prompt_tokens").and_then(Value::as_u64));
    let completion_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .or_else(|| usage.get("completion_tokens").and_then(Value::as_u64));
    Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .or_else(|| prompt_tokens.zip(completion_tokens).map(|(a, b)| a + b)),
        prompt_cache_hit_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .or_else(|| usage.get("cached_tokens").and_then(Value::as_u64)),
        prompt_cache_miss_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64),
    }
}

fn merge_usage(target: &mut Usage, next: Usage) {
    if next.prompt_tokens.is_some() {
        target.prompt_tokens = next.prompt_tokens;
    }
    if next.completion_tokens.is_some() {
        target.completion_tokens = next.completion_tokens;
    }
    if next.total_tokens.is_some() {
        target.total_tokens = next.total_tokens;
    }
    if next.prompt_cache_hit_tokens.is_some() {
        target.prompt_cache_hit_tokens = next.prompt_cache_hit_tokens;
    }
    if next.prompt_cache_miss_tokens.is_some() {
        target.prompt_cache_miss_tokens = next.prompt_cache_miss_tokens;
    }
}

#[derive(Default)]
struct KimiStreamingBlock {
    block_type: String,
    id: Option<String>,
    name: Option<String>,
    text: String,
    thinking: String,
    input: Option<Value>,
    input_json: String,
}

#[derive(Default)]
struct KimiStreamingAccumulator {
    blocks: Vec<KimiStreamingBlock>,
    usage: Usage,
}

impl KimiStreamingAccumulator {
    fn ensure_block(&mut self, index: usize) -> &mut KimiStreamingBlock {
        while self.blocks.len() <= index {
            self.blocks.push(KimiStreamingBlock::default());
        }
        &mut self.blocks[index]
    }

    fn into_response(self) -> Result<ChatResponse> {
        let mut content = String::new();
        let mut reasoning_content = String::new();
        let mut tool_calls = Vec::new();

        for (index, block) in self.blocks.into_iter().enumerate() {
            match block.block_type.as_str() {
                "text" => content.push_str(&block.text),
                "thinking" => reasoning_content.push_str(&block.thinking),
                "tool_use" => {
                    let name = block
                        .name
                        .filter(|name| !name.is_empty())
                        .ok_or_else(|| anyhow!("Kimi streamed tool_use block missing name"))?;
                    let arguments = if !block.input_json.trim().is_empty() {
                        serde_json::from_str(&block.input_json).with_context(|| {
                            format!("failed to parse streamed input for Kimi tool call `{name}`")
                        })?
                    } else {
                        block
                            .input
                            .unwrap_or_else(|| Value::Object(Default::default()))
                    };
                    tool_calls.push(ToolCall {
                        id: block.id.unwrap_or_else(|| format!("toolu_{index}")),
                        call_type: "function".to_string(),
                        function: ToolCallFunction { name, arguments },
                    });
                }
                _ => {
                    if !block.text.is_empty() {
                        content.push_str(&block.text);
                    }
                    if !block.thinking.is_empty() {
                        reasoning_content.push_str(&block.thinking);
                    }
                }
            }
        }

        Ok(ChatResponse {
            content: (!content.is_empty()).then_some(content),
            reasoning_content: (!reasoning_content.is_empty()).then_some(reasoning_content),
            tool_calls,
            usage: self.usage,
        })
    }
}

async fn collect_kimi_anthropic_streaming_chat_response(
    response: reqwest::Response,
    mut on_event: Option<StreamEventCallback<'_>>,
) -> Result<ChatResponse> {
    let mut events = KimiStreamingAccumulator::default();
    let mut buffer = String::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read Kimi provider stream")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(index) = buffer.find('\n') {
            let line = buffer[..index].trim().to_string();
            buffer = buffer[index + 1..].to_string();
            let mut emitted = Vec::new();
            let done = parse_kimi_anthropic_sse_line(&line, &mut events, &mut emitted)?;
            if let Some(callback) = on_event.as_mut() {
                for event in emitted {
                    callback(event);
                }
            }
            if done {
                return events.into_response();
            }
        }
    }
    events.into_response()
}

fn parse_kimi_anthropic_sse_line(
    line: &str,
    events: &mut KimiStreamingAccumulator,
    emitted: &mut Vec<StreamEvent>,
) -> Result<bool> {
    if !line.starts_with("data:") {
        return Ok(false);
    }
    let payload = line.trim_start_matches("data:").trim();
    if payload == "[DONE]" {
        return Ok(true);
    }

    let value: Value =
        serde_json::from_str(payload).context("failed to parse Kimi provider SSE chunk")?;
    if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
        return Err(anyhow!(
            "Kimi streaming request failed: {}",
            redact_secret(error)
        ));
    }

    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "message_start" => {
            if let Some(usage) = value.pointer("/message/usage") {
                merge_usage(&mut events.usage, kimi_usage_from_value(usage));
            }
        }
        "content_block_start" => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let content_block = value.get("content_block").unwrap_or(&Value::Null);
            let block = events.ensure_block(index);
            block.block_type = content_block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            block.id = content_block
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            block.name = content_block
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            block.input = content_block.get("input").cloned();
            if let Some(text) = content_block.get("text").and_then(Value::as_str) {
                block.text.push_str(text);
                emit_stream_event(emitted, Some(text.to_string()), None, false);
            }
            if let Some(thinking) = content_block.get("thinking").and_then(Value::as_str) {
                block.thinking.push_str(thinking);
            }
            if block.block_type == "tool_use" {
                emitted.push(StreamEvent::tool_delta(StreamToolCallDelta {
                    index,
                    id: block.id.clone(),
                    call_type: Some("function".to_string()),
                    name: block.name.clone(),
                    arguments_delta: block.input.as_ref().map(Value::to_string),
                }));
            }
        }
        "content_block_delta" => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let delta = value.get("delta").unwrap_or(&Value::Null);
            let block = events.ensure_block(index);
            match delta
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "text_delta" => {
                    if block.block_type.is_empty() {
                        block.block_type = "text".to_string();
                    }
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        block.text.push_str(text);
                        emit_stream_event(emitted, Some(text.to_string()), None, false);
                    }
                }
                "thinking_delta" => {
                    if block.block_type.is_empty() {
                        block.block_type = "thinking".to_string();
                    }
                    if let Some(thinking) = delta.get("thinking").and_then(Value::as_str) {
                        block.thinking.push_str(thinking);
                    }
                }
                "input_json_delta" => {
                    if block.block_type.is_empty() {
                        block.block_type = "tool_use".to_string();
                    }
                    if let Some(partial_json) = delta.get("partial_json").and_then(Value::as_str) {
                        block.input_json.push_str(partial_json);
                        emitted.push(StreamEvent::tool_delta(StreamToolCallDelta {
                            index,
                            id: block.id.clone(),
                            call_type: Some("function".to_string()),
                            name: block.name.clone(),
                            arguments_delta: Some(partial_json.to_string()),
                        }));
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let block = events.ensure_block(index);
            if block.block_type == "tool_use" {
                if let Some(call) = kimi_streaming_block_tool_call(index, block)? {
                    emitted.push(StreamEvent::tool_completed(call));
                }
            }
        }
        "message_delta" => {
            if let Some(usage) = value.get("usage") {
                merge_usage(&mut events.usage, kimi_usage_from_value(usage));
            }
        }
        "message_stop" => return Ok(true),
        _ => {}
    }

    Ok(false)
}

fn kimi_streaming_block_tool_call(
    index: usize,
    block: &KimiStreamingBlock,
) -> Result<Option<ToolCall>> {
    if block.block_type != "tool_use" {
        return Ok(None);
    }
    let Some(name) = block.name.as_ref().filter(|name| !name.is_empty()) else {
        return Ok(None);
    };
    let arguments = if !block.input_json.trim().is_empty() {
        serde_json::from_str(&block.input_json).with_context(|| {
            format!("failed to parse streamed input for Kimi tool call `{name}`")
        })?
    } else {
        block
            .input
            .clone()
            .unwrap_or_else(|| Value::Object(Default::default()))
    };
    Ok(Some(ToolCall {
        id: block.id.clone().unwrap_or_else(|| format!("toolu_{index}")),
        call_type: "function".to_string(),
        function: ToolCallFunction {
            name: name.clone(),
            arguments,
        },
    }))
}

async fn collect_kimi_anthropic_stream_events(
    response: reqwest::Response,
) -> Result<Vec<StreamEvent>> {
    let mut events = Vec::new();
    let mut buffer = String::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read Kimi provider stream")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(index) = buffer.find('\n') {
            let line = buffer[..index].trim().to_string();
            buffer = buffer[index + 1..].to_string();
            if let Some(event) = parse_kimi_anthropic_stream_event_line(&line)? {
                let done = event.done;
                events.push(event);
                if done {
                    return Ok(events);
                }
            }
        }
    }
    Ok(events)
}

fn parse_kimi_anthropic_stream_event_line(line: &str) -> Result<Option<StreamEvent>> {
    if !line.starts_with("data:") {
        return Ok(None);
    }
    let payload = line.trim_start_matches("data:").trim();
    if payload == "[DONE]" {
        return Ok(Some(StreamEvent::text(None, None, true)));
    }

    let value: Value =
        serde_json::from_str(payload).context("failed to parse Kimi provider SSE chunk")?;
    if let Some(error) = value.pointer("/error/message").and_then(Value::as_str) {
        return Err(anyhow!(
            "Kimi streaming request failed: {}",
            redact_secret(error)
        ));
    }

    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "content_block_delta" => {
            let delta = value.get("delta").unwrap_or(&Value::Null);
            let content_delta = delta
                .get("text")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let reasoning_delta = delta
                .get("thinking")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            if content_delta.is_some() || reasoning_delta.is_some() {
                Ok(Some(StreamEvent::text(
                    content_delta,
                    reasoning_delta,
                    false,
                )))
            } else {
                Ok(None)
            }
        }
        "message_stop" => Ok(Some(StreamEvent::text(None, None, true))),
        _ => Ok(None),
    }
}

fn normalize_chat_response(raw: &str) -> Result<ChatResponse> {
    let response: OpenAiChatResponse =
        serde_json::from_str(raw).context("failed to parse provider chat response")?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("provider response had no choices"))?;
    Ok(ChatResponse {
        content: choice.message.content,
        reasoning_content: choice.message.reasoning_content,
        tool_calls: choice
            .message
            .tool_calls
            .unwrap_or_default()
            .into_iter()
            .map(normalize_tool_call)
            .collect::<Result<Vec<_>>>()?,
        usage: response.usage.unwrap_or_default(),
    })
}

#[derive(Default)]
struct StreamingToolCall {
    id: String,
    call_type: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct StreamingChatAccumulator {
    content: String,
    reasoning_content: String,
    tool_calls: Vec<StreamingToolCall>,
    usage: Usage,
}

impl StreamingChatAccumulator {
    fn into_response(self) -> Result<ChatResponse> {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .filter(|call| !call.name.is_empty() || !call.arguments.trim().is_empty())
            .enumerate()
            .map(|(index, call)| {
                let arguments = if call.arguments.trim().is_empty() {
                    Value::Object(Default::default())
                } else {
                    serde_json::from_str(&call.arguments).with_context(|| {
                        format!(
                            "failed to parse streamed arguments for tool call `{}`",
                            call.name
                        )
                    })?
                };
                Ok(ToolCall {
                    id: if call.id.is_empty() {
                        format!("call_{index}")
                    } else {
                        call.id
                    },
                    call_type: if call.call_type.is_empty() {
                        "function".to_string()
                    } else {
                        call.call_type
                    },
                    function: ToolCallFunction {
                        name: call.name,
                        arguments,
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ChatResponse {
            content: if self.content.is_empty() {
                None
            } else {
                Some(self.content)
            },
            reasoning_content: if self.reasoning_content.is_empty() {
                None
            } else {
                Some(self.reasoning_content)
            },
            tool_calls,
            usage: self.usage,
        })
    }
}

async fn collect_streaming_chat_response(
    response: reqwest::Response,
    mut on_event: Option<StreamEventCallback<'_>>,
) -> Result<ChatResponse> {
    let mut events = StreamingChatAccumulator::default();
    let mut buffer = String::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed to read provider stream")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(index) = buffer.find('\n') {
            let line = buffer[..index].trim().to_string();
            buffer = buffer[index + 1..].to_string();
            let mut emitted = Vec::new();
            let done = parse_sse_chat_line(&line, &mut events, &mut emitted)?;
            if let Some(callback) = on_event.as_mut() {
                for event in emitted {
                    callback(event);
                }
            }
            if done {
                return events.into_response();
            }
        }
    }
    events.into_response()
}

fn parse_sse_chat_line(
    line: &str,
    events: &mut StreamingChatAccumulator,
    emitted: &mut Vec<StreamEvent>,
) -> Result<bool> {
    if !line.starts_with("data:") {
        return Ok(false);
    }
    let payload = line.trim_start_matches("data:").trim();
    if payload == "[DONE]" {
        return Ok(true);
    }
    let value: Value =
        serde_json::from_str(payload).context("failed to parse provider SSE chunk")?;
    if let Some(usage) = value.get("usage") {
        if !usage.is_null() {
            events.usage = serde_json::from_value(usage.clone())
                .context("failed to parse provider SSE usage")?;
        }
    }
    let Some(delta) = value.pointer("/choices/0/delta") else {
        return Ok(false);
    };
    let content_delta = delta.get("content").and_then(Value::as_str);
    let reasoning_delta = delta.get("reasoning_content").and_then(Value::as_str);
    if let Some(content) = content_delta {
        events.content.push_str(content);
    }
    if let Some(reasoning) = reasoning_delta {
        events.reasoning_content.push_str(reasoning);
    }
    emit_stream_event(
        emitted,
        content_delta.map(ToString::to_string),
        reasoning_delta.map(ToString::to_string),
        false,
    );
    if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for call in tool_calls {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            while events.tool_calls.len() <= index {
                events.tool_calls.push(StreamingToolCall::default());
            }
            let target = &mut events.tool_calls[index];
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let call_type = call
                .get("type")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(ToString::to_string);
            let arguments_delta = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                target.id = id.to_string();
            }
            if let Some(call_type) = call.get("type").and_then(Value::as_str) {
                target.call_type = call_type.to_string();
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                if !name.is_empty() {
                    target.name = name.to_string();
                }
            }
            if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
                target.arguments.push_str(arguments);
            }
            emitted.push(StreamEvent::tool_delta(StreamToolCallDelta {
                index,
                id,
                call_type,
                name,
                arguments_delta,
            }));
        }
    }
    Ok(false)
}

fn normalize_tool_call(call: OpenAiToolCall) -> Result<ToolCall> {
    let arguments = if call.function.arguments.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(&call.function.arguments).with_context(|| {
            format!(
                "failed to parse arguments for tool call `{}`",
                call.function.name
            )
        })?
    };
    Ok(ToolCall {
        id: call.id,
        call_type: call.call_type,
        function: ToolCallFunction {
            name: call.function.name,
            arguments,
        },
    })
}

fn parse_sse_line(line: &str) -> Result<Option<StreamEvent>> {
    if !line.starts_with("data:") {
        return Ok(None);
    }
    let payload = line.trim_start_matches("data:").trim();
    if payload == "[DONE]" {
        return Ok(Some(StreamEvent::text(None, None, true)));
    }
    let value: Value =
        serde_json::from_str(payload).context("failed to parse provider SSE chunk")?;
    let delta = value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let reasoning_delta = value
        .pointer("/choices/0/delta/reasoning_content")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    Ok(Some(StreamEvent::text(delta, reasoning_delta, false)))
}

fn emit_stream_event(
    emitted: &mut Vec<StreamEvent>,
    content_delta: Option<String>,
    reasoning_delta: Option<String>,
    done: bool,
) {
    let content_delta = content_delta.filter(|delta| !delta.is_empty());
    let reasoning_delta = reasoning_delta.filter(|delta| !delta.is_empty());
    if content_delta.is_none() && reasoning_delta.is_none() && !done {
        return;
    }
    emitted.push(StreamEvent::text(content_delta, reasoning_delta, done));
}

fn redact_secret(value: &str) -> String {
    let mut output = value.to_string();
    for key in ["apiKey", "api_key", "Authorization", "Bearer"] {
        if output.contains(key) {
            output = output.replace(key, "<redacted>");
        }
    }
    output
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status.is_server_error()
}

fn is_retryable_reqwest(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn is_retryable_provider_body(raw: &str) -> bool {
    raw.trim().is_empty()
}

fn provider_max_attempts() -> usize {
    env::var("DEEPCLI_PROVIDER_MAX_ATTEMPTS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_PROVIDER_ATTEMPTS)
}

fn provider_max_output_tokens() -> usize {
    env::var("DEEPCLI_PROVIDER_MAX_OUTPUT_TOKENS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(8192)
}

fn provider_streaming_chat_enabled() -> bool {
    env::var("DEEPCLI_PROVIDER_STREAMING_CHAT")
        .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "off" | "OFF"))
        .unwrap_or(true)
}

fn retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(250 * 2_u64.pow(attempt.min(5) as u32))
}

fn configured_no_proxy(entries: &[String]) -> Option<reqwest::NoProxy> {
    normalized_no_proxy_list(entries).and_then(|entries| reqwest::NoProxy::from_string(&entries))
}

fn normalized_no_proxy_list(entries: &[String]) -> Option<String> {
    let entries = entries
        .iter()
        .flat_map(|entry| entry.split(','))
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect::<Vec<_>>();
    if entries.is_empty() {
        None
    } else {
        Some(entries.join(","))
    }
}

fn capability_name(capability: ProviderCapability) -> &'static str {
    match capability {
        ProviderCapability::Streaming => "streaming",
        ProviderCapability::Reasoner => "reasoner",
        ProviderCapability::ToolCalling => "tool_calling",
        ProviderCapability::JsonOutput => "json_output",
        ProviderCapability::ContextCache => "context_cache",
    }
}

fn estimate_tokens(messages: &[ProviderMessage]) -> usize {
    messages
        .iter()
        .map(|message| {
            message.role.len().div_ceil(4)
                + message
                    .content
                    .as_deref()
                    .unwrap_or_default()
                    .chars()
                    .count()
                    .div_ceil(4)
                + message
                    .name
                    .as_deref()
                    .map(|name| name.len().div_ceil(4))
                    .unwrap_or_default()
                + 4
        })
        .sum()
}

fn serialize_tool_arguments<S>(value: &Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let text = serde_json::to_string(value).map_err(serde::ser::Error::custom)?;
    serializer.serialize_str(&text)
}

fn deserialize_tool_arguments<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(text) => {
            if text.trim().is_empty() {
                Ok(Value::Object(Default::default()))
            } else {
                serde_json::from_str(&text).map_err(serde::de::Error::custom)
            }
        }
        other => Ok(other),
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiToolCallFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_openai_compatible_tool_call() {
        let raw = r#"{
          "choices": [{
            "message": {
              "content": null,
              "reasoning_content": "thinking",
              "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                  "name": "read_file",
                  "arguments": "{\"path\":\"Cargo.toml\"}"
                }
              }]
            }
          }],
          "usage": {
            "prompt_tokens": 2,
            "completion_tokens": 3,
            "total_tokens": 5,
            "prompt_cache_hit_tokens": 1,
            "prompt_cache_miss_tokens": 1
          }
        }"#;

        let normalized = normalize_chat_response(raw).unwrap();
        assert_eq!(normalized.reasoning_content.as_deref(), Some("thinking"));
        assert_eq!(normalized.tool_calls[0].function.name, "read_file");
        assert_eq!(
            normalized.tool_calls[0].function.arguments["path"],
            "Cargo.toml"
        );
        assert_eq!(normalized.usage.total_tokens, Some(5));
        assert_eq!(normalized.usage.prompt_cache_hit_tokens, Some(1));
        assert_eq!(normalized.usage.prompt_cache_miss_tokens, Some(1));
    }

    #[test]
    fn parses_sse_content_delta() {
        let line = r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#;
        let event = parse_sse_line(line).unwrap().unwrap();
        assert_eq!(event.content_delta.as_deref(), Some("hello"));
        assert_eq!(event.reasoning_delta, None);
        assert!(!event.done);
    }

    #[test]
    fn parses_streaming_chat_line_emits_content_delta() {
        let mut acc = StreamingChatAccumulator::default();
        let mut emitted = Vec::new();

        parse_sse_chat_line(
            r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();

        assert_eq!(acc.content, "hello");
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].content_delta.as_deref(), Some("hello"));
        assert!(!emitted[0].done);
    }

    #[test]
    fn parses_streamed_tool_call_chunks() {
        let mut acc = StreamingChatAccumulator::default();
        let mut emitted = Vec::new();
        parse_sse_chat_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"pa"}}]}}]}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();
        parse_sse_chat_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"Cargo.toml\"}"}}]}}]}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();
        assert!(parse_sse_chat_line("data: [DONE]", &mut acc, &mut emitted).unwrap());
        assert!(emitted.iter().all(|event| event.content_delta.is_none()));
        let response = acc.into_response().unwrap();
        assert_eq!(response.tool_calls[0].id, "call_1");
        assert_eq!(response.tool_calls[0].function.name, "read_file");
        assert_eq!(
            response.tool_calls[0].function.arguments["path"],
            "Cargo.toml"
        );
    }

    #[test]
    fn streamed_tool_call_chunks_emit_protocol_events() {
        let mut acc = StreamingChatAccumulator::default();
        let mut emitted = Vec::new();
        parse_sse_chat_line(
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"Cargo.toml\"}"}}]}}]}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();

        let tool_delta = emitted
            .iter()
            .find_map(|event| event.tool_call_delta.as_ref())
            .expect("stream should expose tool call delta");
        assert_eq!(tool_delta.index, 0);
        assert_eq!(tool_delta.id.as_deref(), Some("call_1"));
        assert_eq!(tool_delta.name.as_deref(), Some("read_file"));
        assert_eq!(
            tool_delta.arguments_delta.as_deref(),
            Some("{\"path\":\"Cargo.toml\"}")
        );
    }

    #[test]
    fn maps_kimi_anthropic_tool_use_response() {
        let raw = r#"{
          "content": [
            {"type":"text","text":"checking"},
            {"type":"tool_use","id":"toolu_1","name":"read_file","input":{"path":"Cargo.toml"}}
          ],
          "usage": {"input_tokens": 10, "output_tokens": 5}
        }"#;

        let response = normalize_kimi_anthropic_response(raw).unwrap();
        assert_eq!(response.content.as_deref(), Some("checking"));
        assert_eq!(response.tool_calls[0].id, "toolu_1");
        assert_eq!(response.tool_calls[0].function.name, "read_file");
        assert_eq!(
            response.tool_calls[0].function.arguments["path"],
            "Cargo.toml"
        );
        assert_eq!(response.usage.total_tokens, Some(15));
    }

    #[test]
    fn parses_kimi_anthropic_streamed_text() {
        let mut acc = KimiStreamingAccumulator::default();
        let mut emitted = Vec::new();
        parse_kimi_anthropic_sse_line(
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":11,"output_tokens":0}}}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();
        parse_kimi_anthropic_sse_line(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();
        parse_kimi_anthropic_sse_line(
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"OK"}}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();
        parse_kimi_anthropic_sse_line(
            r#"data: {"type":"message_delta","usage":{"input_tokens":11,"output_tokens":4,"total_tokens":15}}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();
        assert!(parse_kimi_anthropic_sse_line(
            r#"data: {"type":"message_stop"}"#,
            &mut acc,
            &mut emitted
        )
        .unwrap());

        let response = acc.into_response().unwrap();
        assert_eq!(response.content.as_deref(), Some("OK"));
        assert_eq!(response.usage.total_tokens, Some(15));
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].content_delta.as_deref(), Some("OK"));
    }

    #[test]
    fn parses_kimi_anthropic_streamed_tool_use() {
        let mut acc = KimiStreamingAccumulator::default();
        let mut emitted = Vec::new();
        parse_kimi_anthropic_sse_line(
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_file","input":{}}}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();
        let first_delta = format!(
            "data: {}",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {
                    "type": "input_json_delta",
                    "partial_json": "{\"path\":\"Car"
                }
            })
        );
        parse_kimi_anthropic_sse_line(&first_delta, &mut acc, &mut emitted).unwrap();
        let second_delta = format!(
            "data: {}",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {
                    "type": "input_json_delta",
                    "partial_json": "go.toml\"}"
                }
            })
        );
        parse_kimi_anthropic_sse_line(&second_delta, &mut acc, &mut emitted).unwrap();
        parse_kimi_anthropic_sse_line(
            r#"data: {"type":"content_block_stop","index":0}"#,
            &mut acc,
            &mut emitted,
        )
        .unwrap();
        assert!(emitted.iter().all(|event| event.content_delta.is_none()));
        let completed = emitted
            .iter()
            .find_map(|event| event.tool_call_completed.as_ref())
            .expect("tool_use block stop should emit completed tool call");
        assert_eq!(completed.id, "toolu_1");
        assert_eq!(completed.function.name, "read_file");

        let response = acc.into_response().unwrap();
        assert_eq!(response.tool_calls[0].id, "toolu_1");
        assert_eq!(response.tool_calls[0].function.name, "read_file");
        assert_eq!(
            response.tool_calls[0].function.arguments["path"],
            "Cargo.toml"
        );
    }

    #[test]
    fn parses_kimi_anthropic_stream_event_delta() {
        let event = parse_kimi_anthropic_stream_event_line(
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"OK"}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(event.content_delta.as_deref(), Some("OK"));
        assert!(!event.done);
        assert!(
            parse_kimi_anthropic_stream_event_line(r#"data: {"type":"message_stop"}"#)
                .unwrap()
                .unwrap()
                .done
        );
    }

    #[test]
    fn builds_kimi_anthropic_tool_result_request() {
        let body = kimi_anthropic_request_body(
            "kimi-for-coding",
            &ChatRequest {
                messages: vec![
                    ProviderMessage {
                        role: "system".to_string(),
                        content: Some("system".to_string()),
                        reasoning_content: None,
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                    },
                    ProviderMessage {
                        role: "assistant".to_string(),
                        content: None,
                        reasoning_content: None,
                        name: None,
                        tool_call_id: None,
                        tool_calls: Some(vec![ToolCall {
                            id: "toolu_1".to_string(),
                            call_type: "function".to_string(),
                            function: ToolCallFunction {
                                name: "read_file".to_string(),
                                arguments: json!({"path":"Cargo.toml"}),
                            },
                        }]),
                    },
                    ProviderMessage {
                        role: "tool".to_string(),
                        content: Some("done".to_string()),
                        reasoning_content: None,
                        name: Some("read_file".to_string()),
                        tool_call_id: Some("toolu_1".to_string()),
                        tool_calls: None,
                    },
                ],
                tools: vec![ToolSpec {
                    spec_type: "function".to_string(),
                    function: ToolFunctionSpec {
                        name: "read_file".to_string(),
                        description: "Read".to_string(),
                        parameters: json!({"type":"object"}),
                    },
                }],
                json_mode: false,
            },
            false,
        );

        assert_eq!(body["model"], "kimi-for-coding");
        assert!(body.get("stream").is_none());
        assert_eq!(body["system"], "system");
        assert_eq!(body["messages"][0]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_result");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn builds_kimi_anthropic_stream_request() {
        let body = kimi_anthropic_request_body(
            "kimi-for-coding",
            &ChatRequest {
                messages: vec![ProviderMessage {
                    role: "user".to_string(),
                    content: Some("hello".to_string()),
                    reasoning_content: None,
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                }],
                tools: Vec::new(),
                json_mode: false,
            },
            true,
        );

        assert_eq!(body["stream"], true);
    }

    #[test]
    fn estimates_tokens_without_provider_request() {
        let client = KimiClient::new(ProviderRuntimeConfig {
            name: "kimi".to_string(),
            provider_type: "kimi".to_string(),
            endpoint: None,
            model: Some("kimi".to_string()),
            api_key: None,
            api_id: None,
            capabilities: vec![],
            http_proxy: None,
            https_proxy: None,
            no_proxy: Vec::new(),
        });
        let estimate = client.count_tokens(&[ProviderMessage {
            role: "user".to_string(),
            content: Some("hello world".to_string()),
            reasoning_content: None,
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }]);
        assert!(estimate > 0);
    }

    #[test]
    fn classifies_retryable_provider_status() {
        assert!(is_retryable_status(reqwest::StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(reqwest::StatusCode::BAD_GATEWAY));
        assert!(!is_retryable_status(reqwest::StatusCode::BAD_REQUEST));
    }

    #[test]
    fn normalizes_configured_no_proxy_entries() {
        let entries = vec![
            " localhost, .internal ".to_string(),
            "".to_string(),
            "127.0.0.1".to_string(),
        ];
        assert_eq!(
            normalized_no_proxy_list(&entries).as_deref(),
            Some("localhost,.internal,127.0.0.1")
        );
        assert!(configured_no_proxy(&entries).is_some());
        assert!(configured_no_proxy(&[" ".to_string()]).is_none());
    }

    #[test]
    fn empty_provider_body_is_retryable() {
        assert!(is_retryable_provider_body(""));
        assert!(is_retryable_provider_body("   \n"));
        assert!(!is_retryable_provider_body(r#"{"choices":[]}"#));
    }
}
