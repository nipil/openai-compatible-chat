use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionResponseStream, ChatCompletionStreamOptions, CompletionUsage,
    CreateChatCompletionRequestArgs, CreateChatCompletionStreamResponse, FinishReason, ServiceTier,
};
use portable::{ChatEvent, ChatRequest, Message, MessageRole, OPENAI_CACHE_TOKEN_THRESHOLD};
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString};
use thiserror::Error;
use tracing::{debug, info, trace, warn};

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("failed to build conversation")]
    BuildError { source: OpenAIError },
    #[error("request failed")]
    RequestError { source: OpenAIError },
    #[error("streaming reply failed")]
    StreamingError { source: OpenAIError },
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, AsRefStr,
)]
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

fn get_usage_event(usage: &Option<CompletionUsage>, model_id: &str) -> Option<ChatEvent> {
    let Some(usage) = usage else {
        return None;
    };

    // extract cached token to be able to verify they work (above 1024 token)
    // https://developers.openai.com/api/docs/guides/prompt-caching#requirements
    let cached_tokens = if let Some(details) = &usage.prompt_tokens_details
        && let Some(cached_tokens) = details.cached_tokens
    {
        Some(cached_tokens)
    } else {
        None
    };

    let reasoning_tokens = if let Some(details) = &usage.completion_tokens_details
        && let Some(reasoning_tokens) = details.reasoning_tokens
    {
        Some(reasoning_tokens)
    } else {
        None
    };

    // In-memory prompt cache retention is available for all models that support Prompt Caching,
    // **except for gpt-5.5, gpt-5.5-pro, and all future models.**
    // https://developers.openai.com/api/docs/guides/prompt-caching#in-memory-prompt-cache-retention

    // For gpt-5.5, gpt-5.5-pro, and all future models, the default is 24h and
    // in_memory is not supported. Allowed values are in_memory and 24h.
    // "prompt_cache_retention": "24h"
    // https://developers.openai.com/api/docs/guides/prompt-caching#configure-per-request

    // cache does not seem to activate instantly or on every request (here with gpt-5.4-nano)
    //   https://openai.com/index/api-prompt-caching/
    //   "starting at 1,024 tokens and increasing in 128-token increments"
    // 2026-04-30T12:40:07.401170Z DEBUG native::openai: Token usage prompt=2431 completion=70 total=2501 cached=1792 reasoning=0
    // 2026-04-30T12:40:21.882148Z DEBUG native::openai: Token usage prompt=2516 completion=227 total=2743 cached=1792 reasoning=0
    // 2026-04-30T12:40:35.831326Z DEBUG native::openai: Token usage prompt=2755 completion=184 total=2939 cached=1792 reasoning=0
    // 2026-04-30T12:41:02.590783Z DEBUG native::openai: Token usage prompt=2951 completion=133 total=3084 cached=2816 reasoning=0
    // 2026-04-30T12:41:09.709964Z DEBUG native::openai: Token usage prompt=3096 completion=112 total=3208 cached=2816 reasoning=0
    // 2026-04-30T12:41:12.780628Z DEBUG native::openai: Token usage prompt=3220 completion=130 total=3350 cached=2816 reasoning=0
    // 2026-04-30T12:41:16.567997Z DEBUG native::openai: Token usage prompt=3362 completion=166 total=3528 cached=2816 reasoning=0
    // 2026-04-30T12:41:22.970918Z DEBUG native::openai: Token usage prompt=3540 completion=127 total=3667 cached=2816 reasoning=0
    // 2026-04-30T12:41:27.958606Z DEBUG native::openai: Token usage prompt=3679 completion=169 total=3848 cached=2816 reasoning=0
    // 2026-04-30T12:41:31.220895Z DEBUG native::openai: Token usage prompt=3860 completion=161 total=4021 cached=2816 reasoning=0
    // 2026-04-30T12:41:36.448990Z DEBUG native::openai: Token usage prompt=4033 completion=118 total=4151 cached=3840 reasoning=0
    // 2026-04-30T12:41:44.427819Z DEBUG native::openai: Token usage prompt=4163 completion=126 total=4289 cached=3840 reasoning=0
    // 2026-04-30T12:41:48.827678Z DEBUG native::openai: Token usage prompt=4301 completion=115 total=4416 cached=3840 reasoning=0
    // 2026-04-30T12:41:54.551584Z DEBUG native::openai: Token usage prompt=4428 completion=186 total=4614 cached=3840 reasoning=0
    // 2026-04-30T12:41:58.225582Z DEBUG native::openai: Token usage prompt=4626 completion=138 total=4764 cached=3840 reasoning=0
    // 2026-04-30T12:42:02.018175Z DEBUG native::openai: Token usage prompt=4776 completion=129 total=4905 cached=3840 reasoning=0
    // 2026-04-30T12:42:05.087874Z DEBUG native::openai: Token usage prompt=4917 completion=55 total=4972 cached=3840 reasoning=0
    // 2026-04-30T12:42:10.181740Z DEBUG native::openai: Token usage prompt=4984 completion=74 total=5058 cached=4864 reasoning=0
    // 2026-04-30T12:42:14.052592Z DEBUG native::openai: Token usage prompt=5070 completion=48 total=5118 cached=0 reasoning=0
    // 2026-04-30T12:42:22.598119Z DEBUG native::openai: Token usage prompt=5130 completion=54 total=5184 cached=4864 reasoning=0
    // 2026-04-30T12:42:30.937109Z DEBUG native::openai: Token usage prompt=5196 completion=44 total=5240 cached=4864 reasoning=0
    // 2026-04-30T12:42:50.184715Z DEBUG native::openai: Token usage prompt=5252 completion=43 total=5295 cached=4864 reasoning=0

    // For gpt-3.5-turbo it does not seem to activate, at all ?
    // Some(0) aftr 3k token, even after a few minutes

    // https://www.reddit.com/r/LLMDevs/comments/1p85ko5/i_tested_openais_prompt_caching_across_model/
    // seem that not all model support caching .. and the models in the same family share a cache ?

    debug!(
        prompt = usage.prompt_tokens,
        completion = usage.completion_tokens,
        total = usage.total_tokens,
        cached = cached_tokens,
        reasoning = reasoning_tokens,
        model = model_id,
        "Token usage"
    );

    // Verify that the caching works (cache expires after some minutes)
    if usage.prompt_tokens + usage.completion_tokens > OPENAI_CACHE_TOKEN_THRESHOLD {
        if let Some(cached_tokens) = cached_tokens
            && cached_tokens > 0
        {
            info!(
                efficiency = cached_tokens as f32 / usage.prompt_tokens.max(1) as f32,
                "Cached token hit"
            )
        } else {
            warn!(
                cached = cached_tokens,
                prompt = usage.prompt_tokens,
                "Cached token miss"
            );
        }
    }

    return Some(ChatEvent::TokenCount {
        prompt: usage.prompt_tokens,
        generated: usage.completion_tokens,
        cached: cached_tokens,
        reasoning: reasoning_tokens,
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
    model_id: &str,
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
    if let Some(event) = get_usage_event(&chunk.usage, model_id) {
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
    // do not deduplicate, rely on the provider
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
