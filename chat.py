import json
import os
import sys
import signal
import time
import shutil
import re
import argparse
import logging
from datetime import datetime
from typing import List, Optional

from openai import OpenAI, BadRequestError
from pydantic import BaseModel, Field, ValidationError

from rich.console import Console
from rich.logging import RichHandler
from rich.text import Text
from rich.markdown import Markdown
from rich.live import Live


CONFIG_PATH = "config.json"
MAPPING_PATH = "mapping.json"
EXCLUSION_PATH = "exclusion.json"

ALLOWED_TYPES = {"chat", "multimodal", "reasoning", "instruct"}

console = Console()


# ---------- Logging ----------
logging.basicConfig(
    level=logging.INFO,
    format="%(message)s",
    handlers=[RichHandler(console=console, markup=True, rich_tracebacks=True)],
)
logger = logging.getLogger("cli")
logging.getLogger("httpx").setLevel(logging.WARNING)


def log_debug(msg):
    logger.debug(f"[grey50]{msg}[/grey50]")


def log_info(msg):
    logger.info(f"[white]{msg}[/white]")


def log_warning(msg):
    logger.warning(f"[orange1]{msg}[/orange1]")


def log_error(msg):
    logger.error(f"[red]{msg}[/red]")


def log_critical(msg):
    logger.critical(f"[purple]{msg}[/purple]")


# ---------- Pydantic ----------
class Config(BaseModel):
    api_key: str
    base_url: str
    exclude_model_name_regex: List[str] = Field(default_factory=list)
    prepend_system_prompt: Optional[str] = ""


class ModelMeta(BaseModel):
    family: Optional[str] = None
    type: Optional[str] = None
    max_tokens: Optional[int] = None  # optionnel pour coloration


class Exclusion(BaseModel):
    excluded_models: List[str] = Field(default_factory=list)


# ---------- Load ----------
def load_config():
    try:
        return Config(**json.load(open(CONFIG_PATH)))
    except FileNotFoundError:
        log_error("config.json not found")
    except json.JSONDecodeError as e:
        log_error(f"Invalid JSON in {CONFIG_PATH}: {e}")
    except ValidationError as e:
        log_error(f"Invalid {CONFIG_PATH}:\n{e}")
    sys.exit(1)


def load_mapping():
    if not os.path.exists(MAPPING_PATH):
        return {}
    try:
        raw = json.load(open(MAPPING_PATH))
        return {k: ModelMeta(**v) for k, v in raw.items()}
    except Exception as e:
        log_error(f"Invalid {MAPPING_PATH}: {e}")
        sys.exit(1)


def load_exclusion():
    if not os.path.exists(EXCLUSION_PATH):
        return Exclusion()
    try:
        return Exclusion(**json.load(open(EXCLUSION_PATH)))
    except Exception as e:
        log_error(f"Invalid {EXCLUSION_PATH}: {e}")
        sys.exit(1)


def save_exclusion(ex):
    json.dump(ex.model_dump(), open(EXCLUSION_PATH, "w"), indent=2)


# ---------- Token ----------
def get_token_count(messages, model="gpt-4o"):
    try:
        import tiktoken

        enc = tiktoken.encoding_for_model(model)
        tokens = 0
        for m in messages:
            tokens += 3
            tokens += len(enc.encode(m["content"]))
        tokens += 3
        return tokens
    except Exception:
        return sum(len(m["content"]) // 4 for m in messages)


# ---------- Utils ----------
def now():
    return datetime.now().strftime("%H:%M:%S")


def exit_handler(sig, frame):
    console.print("\n[white]Exiting.[/white]")
    sys.exit(0)


signal.signal(signal.SIGINT, exit_handler)


# ---------- Regex ----------
def compile_regex(patterns):
    compiled = []
    for p in patterns:
        try:
            compiled.append(re.compile(p, re.IGNORECASE))
        except re.error as e:
            log_error(f"Invalid regex: {p} → {e}")
            sys.exit(1)
    return compiled


# ---------- Models ----------
def test_model(client, model):
    try:
        client.models.retrieve(model)
        return True, None
    except BadRequestError as e:
        msg = str(e).lower()
        if "not allowed" in msg or "permission" in msg:
            return False, "not_allowed"
        return False, "invalid"
    except Exception:
        return False, "error"


def list_models(client):
    try:
        return client.models.list().data
    except Exception as e:
        log_error(f"Error fetching models: {e}")
        return []


def explain_model_rejection(model, mapping, exclusion, regex_filters):
    if model in exclusion.excluded_models:
        return "excluded (previously marked as not allowed)"
    if any(r.search(model) for r in regex_filters):
        return "filtered out by exclude_model_name_regex"
    meta = mapping.get(model)
    if meta and meta.type and meta.type not in ALLOWED_TYPES:
        return f"filtered out (type={meta.type} not supported)"
    return "not available in filtered model list"


def enrich(mid, mapping):
    m = mapping.get(mid)
    return {
        "id": mid,
        "family": m.family if m else "zzz",
        "type": m.type if m else None,
        "max_tokens": m.max_tokens if m else None,
    }


def filter_models(models, mapping, excluded, regex):
    out = []
    for m in models:
        if m.id in excluded:
            continue
        if any(r.search(m.id) for r in regex):
            continue
        em = enrich(m.id, mapping)
        if em["type"] is None or em["type"] in ALLOWED_TYPES:
            out.append(em)
    return out


def sort_models(models):
    return sorted(models, key=lambda x: (x["family"], x["id"]))


def display_models(models, excluded):
    entries = []

    for i, m in enumerate(models):
        number = Text(f"{i+1}. ", style="bold")
        name = Text(m["id"], style="white")

        if m["type"]:
            meta = Text(f" ({m['type']})", style="grey50")
            entry = Text.assemble(number, name, meta)
        else:
            entry = Text.assemble(number, name)

        entries.append(entry)

    width = shutil.get_terminal_size((120, 20)).columns
    colw = max(len(e.plain) for e in entries) + 4
    cols = max(1, width // colw)
    rows = (len(entries) + cols - 1) // cols

    for r in range(rows):
        row = []
        for c in range(cols):
            idx = c * rows + r
            if idx < len(entries):
                txt = entries[idx]
                txt.pad_right(colw - len(txt.plain))
                row.append(txt)
        console.print(*row, sep="")

    if excluded:
        console.print(
            "\n[orange1]Ignored models:[/orange1] " + ", ".join(sorted(excluded))
        )


def select_model(models):
    if len(models) == 1:
        log_info(f"Auto-selected: {models[0]['id']}")
        return models[0]["id"]

    display_models(models, [])

    while True:
        c = input("Select model: ")
        if c.isdigit() and 1 <= int(c) <= len(models):
            return models[int(c) - 1]["id"]


# ---------- Chat ----------
def format_token_display(tokens, max_tokens):
    if not max_tokens:
        return Text(f"{tokens}", style="bold white")

    ratio = tokens / max_tokens

    if ratio < 0.5:
        style = "grey50"
    elif ratio < 0.75:
        style = "white"
    elif ratio < 0.9:
        style = "orange1"
    else:
        style = "red"

    return Text(f"{tokens}", style=style)


def chat(client, model, models, exclusion, config):
    console.print("\n[white]--- Conversation ---[/white]\n")

    user_prompt = input("System prompt: ")
    prepend = (config.prepend_system_prompt or "").strip()
    system = (
        prepend + ("\n\n" + user_prompt if user_prompt else "")
        if prepend
        else user_prompt
    )

    history = [{"role": "system", "content": system}]
    closed = False

    model_meta = next((m for m in models if m["id"] == model), {})
    max_tokens = model_meta.get("max_tokens")

    while True:
        tokens = get_token_count(history, model)

        time_txt = Text(f"[{now()}]", style="white")
        model_txt = Text(f"[{model}]", style="italic grey50")
        token_txt = Text("[~") + format_token_display(tokens, max_tokens) + Text("]")

        prompt = Text.assemble(time_txt, model_txt, token_txt, Text("> "))

        msg = console.input(prompt).strip()

        if closed:
            log_warning("Conversation closed. Ctrl-C to exit.")
            continue

        if not msg:
            continue

        history.append({"role": "user", "content": msg})

        try:
            start = time.time()
            stream = client.chat.completions.create(
                model=model, messages=history, stream=True
            )

            full = ""
            last_render = 0

            term_height = shutil.get_terminal_size((120, 40)).lines
            max_live_lines = int(term_height * 0.6)

            use_live = True

            console.print()

            with Live("", console=console, refresh_per_second=10) as live:
                for chunk in stream:
                    delta = chunk.choices[0].delta.content or ""
                    full += delta

                    now_ts = time.time()

                    # throttle render (évite surcoût CPU)
                    if now_ts - last_render < 0.1:
                        continue
                    last_render = now_ts

                    if use_live:
                        try:
                            md = Markdown(full)

                            # estimation hauteur simple
                            lines = full.count("\n") + 1

                            if lines > max_live_lines:
                                use_live = False
                                live.stop()
                                console.print()  # saut propre
                                continue

                            live.update(md)

                        except Exception:
                            use_live = False
                            live.stop()
                            console.print(full)
                            continue

                # fin du stream
                if use_live:
                    live.update(Markdown(full))

            # fallback final (si live désactivé)
            if not use_live:
                try:
                    console.print(Markdown(full))
                except Exception:
                    console.print(full)

            duration = time.time() - start
            console.print(f"[grey50][{duration:.2f}s][/grey50]\n")

            history.append({"role": "assistant", "content": full})

        except BadRequestError as e:
            s = str(e).lower()

            if "context_length" in s:
                log_warning("Context limit reached. Start a new conversation.")
                closed = True
                continue

            if "not allowed" in s:
                log_warning(f"Model {model} not allowed → excluded")

                if model not in exclusion.excluded_models:
                    exclusion.excluded_models.append(model)
                    save_exclusion(exclusion)
                return

            log_error(str(e))


# ---------- Main ----------
def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", help="Directly select model")
    args = parser.parse_args()

    config = load_config()
    mapping = load_mapping()
    exclusion = load_exclusion()

    regex = compile_regex(config.exclude_model_name_regex)

    client = OpenAI(api_key=config.api_key, base_url=config.base_url)

    models = None

    while True:

        if args.model:
            log_info(f"Trying model: {args.model}...")

            ok, reason = test_model(client, args.model)

            if ok:
                rejection = explain_model_rejection(
                    args.model, mapping, exclusion, regex
                )

                if rejection != "not available in filtered model list":
                    log_warning(f"Model '{args.model}' exists but is {rejection}")
                    args.model = None
                else:
                    log_info(f"Using model: {args.model}")
                    chat(client, args.model, [], exclusion, config)
                    return

            elif reason == "not_allowed":
                log_warning(f"Model '{args.model}' not allowed → excluded")

                if args.model not in exclusion.excluded_models:
                    exclusion.excluded_models.append(args.model)
                    save_exclusion(exclusion)

                args.model = None

            else:
                log_error(f"Model '{args.model}' does not exist or is unavailable")
                args.model = None

        if models is None:
            raw = list_models(client)
            models = sort_models(
                filter_models(raw, mapping, exclusion.excluded_models, regex)
            )

        if not models:
            log_critical("No models available")
            return

        model = select_model(models)
        chat(client, model, models, exclusion, config)


if __name__ == "__main__":
    main()
