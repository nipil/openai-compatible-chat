import json
import os
import sys
import signal
import time
import shutil
import re
import argparse
from datetime import datetime
from typing import List, Optional

from openai import OpenAI, BadRequestError
from pydantic import BaseModel, Field, ValidationError


CONFIG_PATH = "config.json"
MAPPING_PATH = "mapping.json"
EXCLUSION_PATH = "exclusion.json"

ALLOWED_TYPES = {"chat", "multimodal", "reasoning", "instruct"}


# ---------- Pydantic ----------
class Config(BaseModel):
    api_key: str
    base_url: str
    exclude_model_name_regex: List[str] = Field(default_factory=list)
    prepend_system_prompt: Optional[str] = ""


class ModelMeta(BaseModel):
    family: Optional[str] = None
    type: Optional[str] = None


class Exclusion(BaseModel):
    excluded_models: List[str] = Field(default_factory=list)


# ---------- Load ----------
def load_config():
    try:
        return Config(**json.load(open(CONFIG_PATH)))
    except FileNotFoundError:
        print("config.json not found")
    except json.JSONDecodeError as e:
        print(f"Invalid JSON in {CONFIG_PATH}: {e}")
    except ValidationError as e:
        print(f"Invalid {CONFIG_PATH}:")
        print(e)
    sys.exit(1)


def load_mapping():
    if not os.path.exists(MAPPING_PATH):
        return {}
    try:
        raw = json.load(open(MAPPING_PATH))
        return {k: ModelMeta(**v) for k, v in raw.items()}
    except Exception as e:
        print(f"Invalid {MAPPING_PATH}: {e}")
        sys.exit(1)


def load_exclusion():
    if not os.path.exists(EXCLUSION_PATH):
        return Exclusion()
    try:
        return Exclusion(**json.load(open(EXCLUSION_PATH)))
    except Exception as e:
        print(f"Invalid {EXCLUSION_PATH}: {e}")
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
    print("\nExiting.")
    sys.exit(0)


signal.signal(signal.SIGINT, exit_handler)


# ---------- Regex ----------
def compile_regex(patterns):
    compiled = []
    for p in patterns:
        try:
            compiled.append(re.compile(p, re.IGNORECASE))
        except re.error as e:
            print(f"Invalid regex: {p} → {e}")
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
        print(f"Error fetching models: {e}")
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
    entries = [
        f"{i+1}. {m['id']} ({m['type']})" if m["type"] else f"{i+1}. {m['id']}"
        for i, m in enumerate(models)
    ]

    width = shutil.get_terminal_size((120, 20)).columns
    colw = max(len(e) for e in entries) + 4
    cols = max(1, width // colw)
    rows = (len(entries) + cols - 1) // cols

    for r in range(rows):
        print(
            "".join(
                entries[c * rows + r].ljust(colw)
                for c in range(cols)
                if c * rows + r < len(entries)
            )
        )

    if excluded:
        print("\nIgnored models:", ", ".join(sorted(excluded)))


def select_model(models):
    if len(models) == 1:
        print(f"Auto-selected: {models[0]['id']}")
        return models[0]["id"]

    display_models(models, [])

    while True:
        c = input("Select model: ")
        if c.isdigit() and 1 <= int(c) <= len(models):
            return models[int(c) - 1]["id"]


# ---------- Chat ----------
def chat(client, model, models, exclusion, config):
    print("\n--- Conversation ---\n")

    user_prompt = input("System prompt: ")
    prepend = (config.prepend_system_prompt or "").strip()
    system = (
        prepend + ("\n\n" + user_prompt if user_prompt else "")
        if prepend
        else user_prompt
    )

    history = [{"role": "system", "content": system}]
    closed = False

    while True:
        tokens = get_token_count(history, model)
        msg = input(f"[{now()}][{model}][~{tokens}]> ").strip()

        if closed:
            print("Conversation closed. Ctrl-C to exit.")
            continue

        if not msg:
            continue

        history.append({"role": "user", "content": msg})

        try:
            start = time.time()
            stream = client.chat.completions.create(
                model=model, messages=history, stream=True
            )

            print()
            full = ""
            for chunk in stream:
                delta = chunk.choices[0].delta.content or ""
                print(delta, end="", flush=True)
                full += delta

            print(f"\n[{time.time()-start:.2f}s]\n")
            history.append({"role": "assistant", "content": full})

        except BadRequestError as e:
            s = str(e).lower()

            if "context_length" in s:
                print("\nContext limit reached. Start a new conversation.\n")
                closed = True
                continue

            if "not allowed" in s:
                print(f"\nModel {model} not allowed → excluded\n")
                if model not in exclusion.excluded_models:
                    exclusion.excluded_models.append(model)
                    save_exclusion(exclusion)
                return

            print(e)


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

    models = None  # lazy load

    while True:

        # --- CLI override ---
        if args.model:
            print(f"Trying model: {args.model}...")

            ok, reason = test_model(client, args.model)

            if ok:
                # vérifier filtres locaux
                rejection = explain_model_rejection(
                    args.model, mapping, exclusion, regex
                )

                if rejection != "not available in filtered model list":
                    print(f"Model '{args.model}' exists but is {rejection}")
                    args.model = None  # fallback menu
                else:
                    print(f"Using model: {args.model}")
                    chat(client, args.model, [], exclusion, config)
                    return

            elif reason == "not_allowed":
                print(f"Model '{args.model}' not allowed → excluded")

                if args.model not in exclusion.excluded_models:
                    exclusion.excluded_models.append(args.model)
                    save_exclusion(exclusion)

                args.model = None

            else:
                print(f"Model '{args.model}' does not exist or is unavailable")
                args.model = None

        # --- Load models only if needed ---
        if models is None:
            raw = list_models(client)
            models = sort_models(
                filter_models(raw, mapping, exclusion.excluded_models, regex)
            )

        if not models:
            print("No models available")
            return

        model = select_model(models)
        chat(client, model, models, exclusion, config)


if __name__ == "__main__":
    main()
