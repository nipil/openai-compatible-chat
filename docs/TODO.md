# What remains to be done

## UX

rust-embed for static bundling (use feature flag to disable)

NIX: add a sytemd unit for user so that it auto-starts in web mode

CLI: pre-fill the system prompt and let the user clear it

CLI: add rustyline instead of simple stdin

UX: add a new/clear conversation button which opens a new tab for the same address ?

UX: provide model info to frontend

move configs to XDG (and have it explain what it is)

WEB: favicon pas affichée dans le tab

CFG: add providers (multiple key+url)

CFG: add more provider knobs : env proxy + auth headers + cookies

CFG: allow configuring proxy and auth in reqwest-injected client

## Code quality

make more robust enums vs strings

add context to each anyhow error
