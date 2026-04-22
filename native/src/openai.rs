use async_openai::{
    error::OpenAIError,
    types::chat::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequest, CreateChatCompletionRequestArgs,
    },
};
use portable::{ChatRequest, Message, MessageRole};
use tracing::debug;

// TODO: thiserror OpenAIError ?
pub fn messages_to_api(
    messages: &[Message],
) -> Result<Vec<ChatCompletionRequestMessage>, OpenAIError> {
    debug!(count = messages.len(), "Building messages for upstream api");
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
        // TODO: make thiserror ?
        .collect::<Result<Vec<_>, OpenAIError>>()
}

// TODO: thiserror OpenAiError ?
pub fn msg_to_api(m: &Message) -> Result<ChatCompletionRequestMessage, OpenAIError> {
    Ok(match m.role {
        MessageRole::System => ChatCompletionRequestSystemMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
        MessageRole::Assistant => ChatCompletionRequestAssistantMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
        MessageRole::User => ChatCompletionRequestUserMessageArgs::default()
            .content(m.content.as_str())
            .build()?
            .into(),
    })
}

// FIXME: do not mix non-openai objects and openai objects
pub fn build_request(
    req: ChatRequest,
    messages: Vec<ChatCompletionRequestMessage>,
) -> Result<CreateChatCompletionRequest, OpenAIError> {
    CreateChatCompletionRequestArgs::default()
        .model(&req.model)
        .messages(messages)
        .build()
}
