# Python → Rust Library Mapping

| Python | Rust | Reason |
| --- | --- | --- |
| `openai.OpenAI` | `async-openai` | First-class async, typed request builders, SSE streaming |
| `pydantic` | `serde` + `anyhow` | Zero-cost deserialization + ergonomic error propagation |
| `argparse` | `clap` (derive) | Compile-time verified CLI, free `--help` / `--version` |
| `rich.Console` + `logging` | `owo-colors` + `eprintln!` | Inline ANSI colour without a framework dependency |
| `rich.Markdown` + `rich.Live` | `termimad` + `crossterm` | Cursor-up/clear re-render loop, same 10 fps throttle policy |
| `rich.Console.input` | `tokio::task::spawn_blocking` + `stdin` | Keeps the async runtime unblocked; readline stays synchronous |
| `dialoguer.Select` (numbered input) | `dialoguer::FuzzySelect` | Strictly better UX: searchable, keyboard-navigable |
| `signal.signal(SIGINT, ...)` | `tokio::signal::ctrl_c()` | Native async signal handling, works on all platforms |
| `tiktoken` (fallback heuristic) | inline heuristic (`chars().count() / 4`) | Mirrors the Python fallback exactly; avoids a heavy dependency |
| `re.compile` | `regex::Regex` | Same semantics, compiled once at startup |
| `json` | `serde_json` | Native Rust JSON, zero-copy where possible |

## Notes

- **Version numbers**: never trust hardcoded crate versions from documentation or LLM output. Always run `cargo add <crate>` to resolve the actual latest version from the registry at build time.
- **`readline` on Windows**: the Python code silently drops `readline` on Windows (`try/except ImportError`). The Rust version uses plain `stdin().read_line()` everywhere, giving identical behaviour on all platforms with no silent feature degradation.

```rust
#[cfg(windows)]
crossterm::execute!(
    std::io::stdout(),
    crossterm::terminal::EnableVirtualTerminalProcessing
).ok();
```
