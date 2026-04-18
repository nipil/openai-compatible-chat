# What remains to be done

## UX

rust-embed for static bundling (use feature flag to disable)

NIX: add a sytemd unit for user so that it auto-starts in web mode

UX: compare raw list of models fetched with known mapping, warn about unknown models, and advice to update mapping file

UX: mapping file is obsolete. Try to get info from API or remove the feature.

CLI: pre-fill the system prompt and let the user clear it

CLI: add rustyline instead of simple stdin

UX: add a new conversation button which opens a new tab for the same address ?

UX: add a clear conversation which clears history and reloads tab ?

move configs to XDG (and have it explain what it is)

WEB: favicon pas affichée dans le tab

CFG: add providers (multiple key+url)

CFG: add more provider knobs : env proxy + auth headers + cookies

use a self-configured reqwest client, as async_openai uses internally, but without default features (including system-proxy), as it selects only json/multipart/

## Code quality

make more robust enums vs strings

## Understand

Abort/stop: AbortController is wrapped in send_wrapper::SendWrapper to satisfy Leptos 0.7's RwSignal<T: Send+Sync> requirement (safe here since WASM is single-threaded)

finalize() guard: both on_done and stop() call the same finalize helper, which guards on streaming to prevent double-execution.
