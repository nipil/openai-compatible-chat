# TODO

CorsLayer::permissive: only for dev environment where `trunk serve` is on a different port than `cargo run --web`. It sets Access-Control-Allow-Origin: *, meaning any website on the internet can make requests to your API from a browser. For a personal local tool this is harmless, but if you ever expose it publicly, a malicious site could use a visitor's browser to hit your API and consume your OpenAI credits.

For a personal server, replacing it swith CorsLayer::new().allow_origin("<http://localhost:PORT".parse>::<HeaderValue>().unwrap()) is the right move. Since Axum will serve the frontend on the same origin as the API, you actually don't need CORS at all in production — you can remove the layer entirely and only add it back for local dev where Trunk's dev server runs on a different port.
TODO: switch to same origin for CORS release (make config-able ?)

TODO: rust-embed for static bundling

TODO: sessionStorage (survives F5, dies when tab closes)

FINAL BUILD ORDER: trunk build --release && cargo build --release

TODO: add a sytemd unit for user so that it auto-starts in web mode

TODO CLI: pre-fill the system prompt and let the user clear it

TODO: check/understand

- Abort/stop: AbortController is wrapped in send_wrapper::SendWrapper to satisfy Leptos 0.7's RwSignal<T: Send+Sync> requirement (safe here since WASM is single-threaded)
- finalize() guard: both on_done and stop() call the same finalize helper, which guards on streaming to prevent double-execution.

TODO CLI: add rustyline instead of simple stdin

TODO: move configs to XDG (and have it explain what it is)

TODO: keep DRY between frontend and backend

TOOD WEB: favicon pas affichée dans le tab

TODO WEB: respect formatting ? verify web markdown rendering too
