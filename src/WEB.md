# Design

## Backend: Axum

It's the simplest, most modern Rust web framework. Lightweight, built on Tokio, and has excellent support for streaming responses via SSE (Server-Sent Events). It will serve two purposes: proxying requests to OpenAI (keeping your API key server-side), and serving the compiled WASM frontend as static files.

## Frontend: Leptos

The best choice for a simple reactive SPA in Rust/WASM right now. It has a clean component model, handles async and reactive state elegantly, and its compiled output is very small. You won't need a router, so you'll only use its core reactivity and component system.

## Build tooling: Trunk

The standard tool for building and bundling Rust WASM frontends. It handles the WASM compilation, asset pipeline, and dev server with hot-reload out of the box. Zero config for a simple project like this.

The streaming flow will be: Leptos frontend sends a fetch request → Axum backend forwards it to OpenAI with streaming enabled → Axum streams tokens back as SSE → Leptos reads the SSE stream and appends tokens to the UI reactively.

That's the entire stack: Axum + async-openapi on the back, Leptos + Trunk on the front. No database, no auth middleware, no extra complexity.

## API Reference

Base URL: `http://localhost:3000`

---

### GET /api/models

Returns the list of available models, filtered and sorted according to the loaded configuration (exclusion list, regex filters, allowed types).

    curl http://localhost:3000/api/models | jq

### Response

`200 OK` — `application/json`

```json
[
  {
    "id": "gpt-4o",
    "family": "gpt-4",
    "model_type": "chat",
    "max_tokens": 128000
  }
]
```

| Field | Type | Nullable | Description |
|---|---|---|---|
| `id` | string | no | Model identifier as returned by the API |
| `family` | string | no | Model family from the local mapping |
| `model_type` | string | yes | One of `chat`, `multimodal`, `reasoning`, `instruct` |
| `max_tokens` | number | yes | Maximum context size from the local mapping |

Models not present in the local mapping, excluded by the exclusion list, or matching the configured regex filters are omitted. The list reflects the current in-memory exclusion list, which may have been updated at runtime by a previous unauthorized chat request.

---

### POST /api/chat

Sends a full conversation history to the configured OpenAI-compatible API and streams the assistant's reply token by token as Server-Sent Events.

The backend is stateless. The client is responsible for maintaining the conversation history and sending it in full on every request.

    curl -N -X POST http://localhost:3000/api/chat \
      -H "Content-Type: application/json" \
      -d '{
        "model": "gpt-4o",
        "messages": [
          { "role": "user",      "content": "My name is Alice." },
          { "role": "assistant", "content": "Nice to meet you, Alice!" },
          { "role": "user",      "content": "What is my name?" }
        ]
      }'

#### Request

`Content-Type: application/json`

```json
{
  "model": "gpt-4o",
  "messages": [
    { "role": "user",      "content": "Hello!" },
    { "role": "assistant", "content": "Hi, how can I help?" },
    { "role": "user",      "content": "What is Rust?" }
  ]
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `model` | string | yes | Model ID to use, must match an ID from `GET /api/models` |
| `messages` | array | yes | Full conversation history, ordered oldest to newest |
| `messages[].role` | string | yes | One of `user`, `assistant`, `system` |
| `messages[].content` | string | yes | Message text |

**System prompt:** if a `prepend_system_prompt` is configured server-side and the first message in the array is not a `system` message, the server automatically prepends one. There is no need to send it from the client.

#### Response

`200 OK` — `text/event-stream`

The response is a stream of Server-Sent Events. Each event carries one token (delta) of the assistant's reply.

**Token event** (normal flow):

```
data: Hello
data: , here
data:  is an explanation
```

**Error event** (API or network failure):

```
event: error
data: <error message>
```

If the error indicates the requested model is not accessible, the server will additionally add that model to the exclusion list, persist it to disk, and remove it from future `GET /api/models` responses.

#### Notes

- The stream ends when the SSE connection is closed by the server. There is no explicit `[DONE]` event.
- An error event does not necessarily close the stream; the client should treat any `error` event as terminal and close the connection.
- Sending a `system` message as the first element of `messages` overrides the server-side system prompt entirely.
