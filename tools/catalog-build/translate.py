#!/usr/bin/env python3
"""Offline machine translation of catalog names (no web API, no data leaves
the runner).

    translate.py NAMES.tsv translations/es.tsv [--model-cache DIR]

NAMES.tsv comes from `catalog-build names`. Rows already in the target file
are kept when they are `human` (hand-written or corrected: never touched) or
`machine` with an unchanged English source; every other name is translated
with the Argos Translate en→es model (OPUS data, CTranslate2 runtime) and
written as `machine`. Rows whose id left the catalog are dropped.

Dependencies: ctranslate2, sentencepiece.
"""

import argparse
import re
import shutil
import hashlib
import sys
import urllib.request
import zipfile
from pathlib import Path

MODELS = {
    "es": (
        "https://argos-net.com/v1/translate-en_es-1_0.argosmodel",
        "d698d0ef87ad70d5d184b7fa6965905bf4368f09a2bb9ffb165a79bac96af0c4",
    ),
}

HEADER = "# id\tEnglish source\ttranslation\torigin (machine | glossary | human)\n"


def model_dir(lang: str, cache: Path) -> Path:
    url, sha256 = MODELS[lang]
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / f"en_{lang}.argosmodel"
    if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != sha256:
        print(f"downloading {url}", file=sys.stderr)
        # The default Python-urllib agent is refused (403) by the model host.
        request = urllib.request.Request(url, headers={"User-Agent": "myos-catalog-build/1"})
        with urllib.request.urlopen(request) as response, archive.open("wb") as out:
            shutil.copyfileobj(response, out)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        if digest != sha256:
            sys.exit(f"{url} changed upstream (SHA-256 {digest}): review it and update the pin")
    target = cache / f"en_{lang}"
    if not (target / "model" / "model.bin").exists():
        with zipfile.ZipFile(archive) as zipped:
            zipped.extractall(cache)
    return target


def read_table(path: Path) -> dict:
    rows = {}
    if not path.exists():
        return rows
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 4 or fields[3] not in ("machine", "glossary", "human"):
            sys.exit(f"{path}:{number}: malformed row")
        rows[fields[0]] = fields
    return rows


def clean(text: str) -> str:
    text = " ".join(text.replace("\t", " ").split())
    return text[:1].upper() + text[1:]


def prepare(text: str) -> str:
    """Lowercase Title Case words (the model copies capitalised phrases as
    proper names); ALL-CAPS words (brands) stay."""
    return " ".join(
        word if (word.isupper() and len(word) > 1) else word.lower() for word in text.split()
    )


def load_glossary(path: Path) -> dict:
    """Phrase (tuple of words) → (role, spanish, gender)."""
    glossary = {}
    if not path.exists():
        return glossary
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        role, english = fields[0], fields[1]
        spanish = fields[2] if len(fields) > 2 else ""
        gender = fields[3] if len(fields) > 3 else ""
        if role not in ("move", "of", "mod", "gear", "skip") or (role == "move" and gender not in ("m", "f", "mp", "fp")):
            sys.exit(f"{path}:{number}: malformed glossary row")
        glossary[tuple(english.split())] = (role, spanish, gender)
    return glossary


def inflect(adjective: str, gender: str) -> str:
    """Masculine singular adjective → the move's gender and number."""
    if " " in adjective:
        return adjective
    word = adjective
    if gender.startswith("f") and word.endswith("o"):
        word = word[:-1] + "a"
    if gender.endswith("p"):
        word += "s" if word[-1] in "aeiou" else "es"
    return word


def compose(name: str, glossary: dict) -> str | None:
    """Exercise name from glossary phrases, or None when a word is not
    covered (a wrong translation is worse than the English name)."""
    words = re.sub(r"[^a-z0-9 ]", " ", name.lower().replace("'", "")).split()
    moves, targets, mods, gear = [], [], [], []
    at = 0
    while at < len(words):
        for length in range(min(5, len(words) - at), 0, -1):
            entry = glossary.get(tuple(words[at : at + length]))
            if entry:
                role, spanish, gender = entry
                {"move": moves, "of": targets, "mod": mods, "gear": gear, "skip": []}[role].append(
                    (spanish, gender)
                )
                at += length
                break
        else:
            return None
    if not moves:
        return None
    gender = moves[0][1]
    parts = [" y ".join(spanish for spanish, _ in moves)]
    parts += [spanish for spanish, _ in targets]
    parts += [inflect(spanish, gender) for spanish, _ in mods]
    # One equipment phrase per kind is enough ("con barra en banco" is fine).
    parts += list(dict.fromkeys(spanish for spanish, _ in gear))
    text = " ".join(parts)
    return text[:1].upper() + text[1:]


def translate(names: list, lang: str, cache: Path) -> list:
    import ctranslate2
    import sentencepiece

    directory = model_dir(lang, cache)
    tokenizer = sentencepiece.SentencePieceProcessor(model_file=str(directory / "sentencepiece.model"))
    translator = ctranslate2.Translator(str(directory / "model"), device="cpu", inter_threads=4)
    out = []
    batch = 64
    for start in range(0, len(names), batch):
        chunk = [prepare(name) for name in names[start : start + batch]]
        tokens = tokenizer.encode(chunk, out_type=str)
        results = translator.translate_batch(tokens, beam_size=4, max_decoding_length=96)
        out.extend(clean(tokenizer.decode(result.hypotheses[0])) for result in results)
        print(f"{min(start + batch, len(names))}/{len(names)}", file=sys.stderr)
    return out


def load_food_glossary(path: Path) -> dict:
    glossary = {}
    if not path.exists():
        return glossary
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 2:
            sys.exit(f"{path}:{number}: malformed glossary row")
        glossary[fields[0].lower()] = fields[1]
    return glossary


def translate_foods(names: list, glossary: dict, lang: str, cache: Path) -> list:
    """Each comma-separated segment: glossary, kept as is when written in
    capitals (brands), machine translation otherwise (each distinct segment
    once)."""
    split = [[segment.strip() for segment in name.split(",") if segment.strip()] for name in names]
    unknown = sorted(
        {
            segment
            for segments in split
            for segment in segments
            if segment.lower() not in glossary and not segment.isupper()
        }
    )
    machine = dict(zip(unknown, translate(unknown, lang, cache))) if unknown else {}
    # The model capitalises; keep the source segment's case (proper names
    # stay capitalised, descriptors do not).
    for segment, text in machine.items():
        if segment[:1].islower():
            machine[segment] = text[:1].lower() + text[1:]
    out = []
    for segments in split:
        parts = [
            glossary.get(segment.lower()) or (segment if segment.isupper() else machine[segment])
            for segment in segments
        ]
        text = ", ".join(parts)
        out.append(text[:1].upper() + text[1:])
    print(f"{len(unknown)} distinct segments machine-translated", file=sys.stderr)
    return out


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("names", type=Path)
    parser.add_argument("table", type=Path)
    parser.add_argument("--model-cache", type=Path, default=Path.home() / ".cache" / "myos-mt")
    args = parser.parse_args()

    lang = args.table.stem
    if lang not in MODELS:
        sys.exit(f"no model for {lang}")
    existing = read_table(args.table)
    glossary = load_glossary(args.table.with_name(f"glossary-{lang}.tsv"))
    current, kinds = {}, {}
    for line in args.names.read_text(encoding="utf-8").splitlines():
        identifier, kind, source = line.split("\t")
        current[identifier] = source
        kinds[identifier] = kind

    keep, todo = {}, []
    composed = 0
    for identifier, source in current.items():
        row = existing.get(identifier)
        if row and row[3] == "human":
            keep[identifier] = row
        elif kinds[identifier] == "exercise":
            # Exercises: glossary only (general machine translation mangles
            # gym jargon), recomputed every run so glossary fixes apply.
            name = compose(source, glossary)
            if name:
                keep[identifier] = [identifier, source, name, "glossary"]
                composed += 1
        else:
            todo.append(identifier)
    # Foods: segment by segment, glossary first; recomputed every run so
    # glossary fixes apply (the model only sees what the glossary lacks).
    food_glossary = load_food_glossary(args.table.with_name(f"glossary-food-{lang}.tsv"))
    translated = translate_foods([current[i] for i in todo], food_glossary, lang, args.model_cache)
    for identifier, name in zip(todo, translated):
        keep[identifier] = [identifier, current[identifier], name, "machine"]

    ordered = sorted(keep.values(), key=lambda row: (row[1].lower(), row[0]))
    args.table.parent.mkdir(parents=True, exist_ok=True)
    with args.table.open("w", encoding="utf-8") as out:
        out.write(HEADER)
        for row in ordered:
            out.write("\t".join(row) + "\n")
    dropped = len(set(existing) - set(current))
    exercises = sum(1 for kind in kinds.values() if kind == "exercise")
    print(
        f"{lang}: {composed}/{exercises} exercises from the glossary, {len(todo)} foods translated, "
        f"{len(keep) - len(todo) - composed} kept, {dropped} dropped",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
