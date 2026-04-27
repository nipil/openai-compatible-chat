use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionResponseStream, ChatCompletionStreamOptions, CompletionUsage,
    CreateChatCompletionRequestArgs, CreateChatCompletionStreamResponse, FinishReason, ServiceTier,
};
use portable::{ChatEvent, ChatRequest, Message, MessageRole};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};
use thiserror::Error;
use tracing::{debug, trace, warn};

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("failed to build conversation")]
    BuildError { source: OpenAIError },
    #[error("request failed")]
    RequestError { source: OpenAIError },
    #[error("streaming reply failed")]
    StreamingError { source: OpenAIError },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString)]
#[strum(serialize_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    Audio,
    Chat,
    Completion,
    Embedding,
    Image,
    Instruct,
    Moderation,
    Multimodal,
    Realtime,
    Reasoning,
    Search,
    Transcription,
    Video,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn messages_to_api(
    messages: &[Message],
) -> Result<Vec<ChatCompletionRequestMessage>, ProviderError> {
    debug!(count = messages.len(), "Building openapi messages");
    messages
        .iter()
        // remove empty system messages to avoid confusing the model with empty instructions
        .filter(|m| !(m.role == MessageRole::System && m.content.trim().is_empty()))
        .map(msg_to_api)
        // There are two possible way to collect "list of results" :
        // - collect::<Vec<Result<_,_>>>() → keep every result
        //   → what we would do if we wanted to log each error (for example)
        // - collect::<Result<Vec<_>>>() → first error wins, rest is ignored
        //   → what we do here, as we do nothing like logging each err
        .collect::<Result<Vec<_>, ProviderError>>()
}

fn msg_to_api(m: &Message) -> Result<ChatCompletionRequestMessage, ProviderError> {
    Ok(match m.role {
        MessageRole::System => ChatCompletionRequestSystemMessageArgs::default()
            .content(m.content.as_str())
            .build()
            .map_err(|e| ProviderError::BuildError { source: e })?
            .into(),
        MessageRole::Assistant => ChatCompletionRequestAssistantMessageArgs::default()
            .content(m.content.as_str())
            .build()
            .map_err(|e| ProviderError::BuildError { source: e })?
            .into(),
        MessageRole::User => ChatCompletionRequestUserMessageArgs::default()
            .content(m.content.as_str())
            .build()
            .map_err(|e| ProviderError::BuildError { source: e })?
            .into(),
    })
}

fn get_usage_event(usage: &Option<CompletionUsage>) -> Option<ChatEvent> {
    let Some(usage) = usage else {
        return None;
    };
    debug!(
        prompt = usage.prompt_tokens,
        completion = usage.completion_tokens,
        total = usage.total_tokens,
        "Token usage"
    );
    return Some(ChatEvent::TokenCount {
        prompt: usage.prompt_tokens,
        generated: usage.completion_tokens,
    });
}

fn get_finish_event(reason: &Option<FinishReason>, refusal: &Option<String>) -> Option<ChatEvent> {
    let Some(reason) = reason else {
        return None;
    };
    debug!(reason = ?reason, refusal = ?refusal, "Finish");
    // This does not use strum macros, so we serialize it
    let reason = serde_json::to_string(reason)
        .expect("FinishReason serializing must not fail")
        .trim_matches('"')
        .to_owned();
    return Some(ChatEvent::FinishReason {
        reason,
        refusal: refusal.clone(),
    });
}

pub(crate) fn get_chat_event(
    chunk: Result<CreateChatCompletionStreamResponse, OpenAIError>,
) -> ChatEvent {
    let chunk = match chunk {
        Ok(chunk) => chunk,
        Err(e) => {
            warn!("Server side error while processing chunk: {:?}", e);
            return ChatEvent::Error(e.to_string());
        }
    };

    // log request misconfiguration
    if chunk.choices.len() > 1 {
        warn!(chunk = ?chunk, "choice not unique");
    } else {
        trace!(chunk = ?chunk, "response");
    }

    // usage is the last chunk, with zero choice
    if let Some(event) = get_usage_event(&chunk.usage) {
        return event;
    }

    // only go on if we have a single choice
    let Some(choice) = chunk.choices.get(0) else {
        return ChatEvent::Error("No choice available".into());
    };

    // when there is a finish reason, there is no content
    if let Some(event) = get_finish_event(&choice.finish_reason, &choice.delta.refusal) {
        return event;
    }

    // only go on if we have a content
    let Some(ref content) = choice.delta.content else {
        return ChatEvent::Error("No content".into());
    };

    // send the actual content of the chunk
    trace!(content = content, "content sent to front-end");
    ChatEvent::MessageToken(content.clone())
}

// ── API ───────────────────────────────────────────────────────────────────────

pub async fn list_models(client: &Client<OpenAIConfig>) -> Result<Vec<String>, ProviderError> {
    // TODO: deduplicate ? just in case ?
    client
        .models()
        .list()
        .await
        .map(|r| {
            r.data
                .into_iter()
                .map(|m| {
                    debug!(model = m.id, "List models");
                    m.id
                })
                .collect()
        })
        .map_err(|e| ProviderError::RequestError { source: e })
}

pub(crate) async fn send_chat_request(
    client: &Client<OpenAIConfig>,
    chat: &ChatRequest,
) -> Result<ChatCompletionResponseStream, ProviderError> {
    // TODO: implement pathological cases here if needed (huge payload)
    // TODO: implement message-based busines logic here (logging)

    // build each message then complete conversation
    let messages = messages_to_api(&chat.messages)?;
    let request = CreateChatCompletionRequestArgs::default()
        .model(&chat.model)
        .messages(messages)
        // IMPORTANT: untested
        // FIXME: with Flex ==>  invalid_request_error: Invalid service_tier argument
        // FIXME: with Priority ==> response service_tier=Some(Default)
        .service_tier(ServiceTier::Default)
        // IMPORTANT: there can be 1 or N or 0 response in choices
        // N is alternate possible conversation (user choice or always first)
        // N=1 by default in async_openai, but lets make future proof
        // usually, 0 is for the last, when "include_usage" is true
        .n(1)
        // Enable usage info, which will be in the chunk where delta is empty
        .stream_options(ChatCompletionStreamOptions {
            include_usage: Some(true),
            include_obfuscation: Some(true),
        })
        .build()
        .map_err(|e| ProviderError::BuildError { source: e })?;

    // build and send the request for a streamed answer for this conversation
    trace!(request = ?request, "openai request");
    client
        .chat()
        .create_stream(request)
        .await
        .map_err(|e| ProviderError::RequestError { source: e })
}
