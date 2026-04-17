# TODO

## UX

rust-embed for static bundling (use feature flag to disable)

WEB: sessionStorage (survives F5, dies when tab closes)

NIX: add a sytemd unit for user so that it auto-starts in web mode

CLI: pre-fill the system prompt and let the user clear it

CLI: add rustyline instead of simple stdin

move configs to XDG (and have it explain what it is)

WEB: favicon pas affichée dans le tab

WEB: sessionStorage (survives F5, dies when tab closes)

## Code quality

keep DRY between frontend and backend

make more robust enums vs strings

make consts out of fixes strings which are not variables

## Understand

Abort/stop: AbortController is wrapped in send_wrapper::SendWrapper to satisfy Leptos 0.7's RwSignal<T: Send+Sync> requirement (safe here since WASM is single-threaded)

finalize() guard: both on_done and stop() call the same finalize helper, which guards on streaming to prevent double-execution.
