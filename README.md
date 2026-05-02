# CLI Chatbot (OpenAI-compatible)

## Table of Contents

- [Installation](#installation)
- [Update](#update)
- [Configuration](#configuration)
- [Usage](#usage)
- [Features](#features)
- [Dev Workflow](#dev-workflow)
- [Architecture](#architecture)
- [What's next?](#whats-next)

## Description

Simple command-line and web chatbot written in Rust.

It is compatible with

- any OpenAI-compatible provider (public or private)
- any model supporting `/v1/chat/completions` end points

About the code

- the majority of the code was first written by Claude.ai, and I used ChatGPT for various things
  - i spent about 10% of the time beforehand, drafting a prompt with the design of the product
  - about 75% of the "generate working code" job was done in 10% of the time by Claude
  - i spent 80% of the time ... doing 25% of the "quality work" i enjoy (learn by reworking)

- current state
  - reworked the whole thing to learn about [each component and technology](#architecture)
  - cleaning, refactoring and improving the parts until i was satisfied with its shape
  - error management, which was entirely missing (because i did not request it at first)

## Sample rendering

Web interface

![sample](docs/sample-web.png)

CLI in dark mode

![sample](docs/sample-cli.png)

## Installation

Binaries are automatically generated at each release.

### 📦 Download a binary

1. Go to the [releases page](https://github.com/nipil/openai-compatible-chat/releases)

2. Download the archive for your system:
   - 🪟 Windows: `openai-compatible-chat-x86_64-pc-windows-msvc.zip` (MSVC)
   - 🐧 Linux: `openai-compatible-chat-x86_64-unknown-linux-musl.zip` (static MUSL)
   - 🍎 macOS: `openai-compatible-chat-aarch64-apple-darwin.zip` (Apple Silicon)
   - 🍎 macOS: `openai-compatible-chat-x86_64-apple-darwin.zip` (Intel)
   - 🍎 macOS: `openai-compatible-chat-macos.zip` (universal)

3. Extract the ZIP archive

4. On Linux/macOS: make the binary executable (using `chmod +x`)

5. Run the included executable

- after setting up a [configuration file](#configuration)
- and taking a look at the [usage](#usage)

## Update

Simply download the latest version from the releases page and replace the old binary.

## Configuration

Create a `config.json` file in the same folder as the binary, adapting the example below:

```json
{
  "api_key": "sk-svcacct-************************",
  "base_url": "https://api.openai.com/v1",
  "exclude_model_name_regex": [
    ".*-3.5-turbo-\\d+"
  ],
  "default_system_prompt": "This system prompt will be shown as default for every new session"
}
```

## Usage

```text
Usage: openai-compatible-chat [OPTIONS] <COMMAND>

Commands:
  cli   CLI subcommand
  web   Web subcommand
  help  Print this message or the help of the given subcommand(s)

Options:
  -t, --api-timeout-ms <API_TIMEOUT_MS>  [default: 10000]
  -c, --config-file <CONFIG_FILE>        [default: config.json]
  -i, --info-file <INFO_FILE>            [default: ai_model_info/openai.json]
  -m, --model-lock <MODEL_LOCK>
      --log-file <LOG_FILE>
  -h, --help                             Print help
  -V, --version                          Print version
```

### CLI mode

Start an interactive chat session in your terminal:

```bash
./openai-compatible-chat cli
```

### Web mode

Start the web interface, serving the compiled WASM frontend and proxying API requests:

```bash
./openai-compatible-chat web --port 8080
```

```text
Options:
  -p, --port <PORT>            Port to listen on
  -d, --dist-wasm <DIST_WASM>  Path to WASM dist directory [default: wasm/dist]
```

Then open `http://localhost:8080` in your browser.

### Direct model selection

You can bypass the model selection menu with:

```bash
./openai-compatible-chat --model-lock gpt-4o cli
```

Behavior:

- verifies that the model exists in the list retrieved via the API
- applies filters (exclusions + regex)
- if valid → starts the conversation directly
- otherwise → error message + back to menu

## Features

- interactive model selection
- streaming responses
- disposable history (no storage at all)
- token display (exact or estimated, cache efficiency on info log level)
- model filtering (regex) via configuration
- error handling (forbidden model, context overflow, plus all possible unhappy path)

## Dev Workflow

Proxy: if needed, set the VSCode setting `rust-analyzer.cargo.extraEnv`:

```json
"rust-analyzer.cargo.extraEnv": {
    "ALL_PROXY": "http://10.154.61.6:3128"
}
```

Install prerequisites:

```shell
# required for the rust-analyzer VSCode extension
rustup component add rust-src

# wasm toolchain
rustup target add wasm32-unknown-unknown

# adds the code formatter (only) from nightly
# IMPORTANT: builds are done using stable !
rustup toolchain install nightly
rustup component add rustfmt --toolchain nightly

# tool for hot-building/reloading wasm and static files
cargo install trunk

# tool for hot-building/reloading native code
cargo install watchexec-cli
```

Version management

```shell
# show dupplicated (often, pulled) versions
cargo tree -d --depth 1

# show unused dependencies
cargo +nightly udeps

# unlike "cargo update", shows versions beyond semver
cargo outdated

### Model info

The JSON files in `ai_model_info` must be kept up to date, as they are used as metadata to filter which models to use for each function.

However, this data is not officially and centrally available, and must be periodically updated to add new models (returned by the API) using public data.

The recommended workflow to update model info:

- use [Claude.ai](https://claude.ai) since it has internet access and does the job
- for a model info file that needs updating:
  - extract all **incomplete** models (those with `null` fields) from the JSON file
  - get the list of model *ids* retrieved from the API for which you have no info (see logs)

Paste the batch of incomplete JSON, then submit the prompt below with your list of missing models:

```text
Can you please update my attached incomplete json model metadata compilation
WITH ACCURATE DATA (no hallucinating !!) from up-to-date sources,
for all AI model id listed below, which i just got from the AI provider API

    ```
    gpt-5.4-nano-2026-03-17
    gpt-5.4-mini-2026-03-17
    ```
```

Wait for the result, then paste it back into the original JSON file.

Then run the command below to **pretty-print the JSON**, which makes commits produce a clean diff that can be reviewed to track changes:

```bash
cargo run -p ai_model_info ai_model_info
```

**Review the changes** made, and verify they are "consistent".

Copy Claude's summary of actions (use the "copy" button to get it **in markdown format!**)

Commit, making sure to **archive Claude's explanation** in the commit message.

### Debug

Start the backend (use the port from key `backend` in section `[[proxy]]` of `wasm/Trunk.toml`):

```shell
watchexec --clear --quiet --restart --debounce 1s --stop-signal SIGTERM --ignore "wasm/**" --exts rs cargo run -p native -- web --port 3000
```

Hot-build and reload Rust/WASM code and serve static files:

```shell
cd wasm
watchexec --clear --quiet --restart --debounce 1s --stop-signal SIGTERM --watch "../portable" --exts rs trunk serve
```

Hot-build documentation if needed

```shell
watchexec --clear --quiet --restart --debounce 10s --stop-signal SIGTERM --watch Cargo.lock cargo doc --locked
```

Visit `http://localhost:8080` in your browser.

### Release

```shell
cd wasm && trunk build --release
```

```shell
cd ..
cargo build --release
```

## Architecture

![Components and links](docs/architecture.svg)

That's the entire stack:

- Axum + async-openapi on the back
- Leptos + Trunk on the front.

No database, no auth middleware, no extra complexity.

### Backend: Axum

The simplest, most modern Rust web framework. Lightweight, built on Tokio, and with excellent support for streaming responses via SSE (Server-Sent Events). It serves two purposes: proxying requests to OpenAI (keeping your API key server-side), and serving the compiled WASM frontend as static files.

### Frontend: Leptos

The best choice for a simple reactive SPA in Rust/WASM right now. It has a clean component model, handles async and reactive state elegantly, and its compiled output is very small. No router needed — only core reactivity and the component system are used.

### Build tooling: Trunk

The standard tool for building and bundling Rust WASM frontends. It handles WASM compilation, asset pipeline, and dev server with hot-reload out of the box. Zero config for a simple project like this.

### Application flow

The streaming flow: Leptos frontend sends a fetch request → Axum backend forwards it to OpenAI with streaming enabled → Axum streams tokens back as SSE → Leptos reads the SSE stream and appends tokens to the UI reactively.

## What's next?

There is surely [something fun to do!](docs/TODO.md)
