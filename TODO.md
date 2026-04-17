# TODO

## UX

rust-embed for static bundling (use feature flag to disable)

NIX: add a sytemd unit for user so that it auto-starts in web mode

UX: selected model does not reflect "model" cookie (even if not locked !)

CLI: pre-fill the system prompt and let the user clear it

CLI: add rustyline instead of simple stdin

UX: add a new conversation button which opens a new tab for the same address ?

UX: add a clear conversation which clears history and reloads tab ?

move configs to XDG (and have it explain what it is)

WEB: favicon pas affichée dans le tab

## Code quality

keep DRY between frontend and backend

make more robust enums vs strings

make consts out of fixes strings which are not variables

## Understand

Abort/stop: AbortController is wrapped in send_wrapper::SendWrapper to satisfy Leptos 0.7's RwSignal<T: Send+Sync> requirement (safe here since WASM is single-threaded)

finalize() guard: both on_done and stop() call the same finalize helper, which guards on streaming to prevent double-execution.
