#!/usr/bin/env python3
"""Writes app/src/main/assets/licenses/third_party.json: MissionCtrl itself (LICENSE, licence and
version from core/Cargo.toml) and every library that ships in the app, with its licence text, for
Settings › Open-source licences.

Run after changing dependencies (Rust crates or Gradle):
    python3 android/scripts/third_party_licenses.py

Rust: normal dependencies of mc-ffi for aarch64-linux-android (cargo metadata), each with the
licence files it ships (two folders deep, so vendored C sources like whisper.cpp count too); a
crate without files gets the standard text of its licence from license-texts/. Android: the
release runtime classpath (Gradle). Texts that are the same are stored once.
"""
import hashlib
import json
import os
import re
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ANDROID = os.path.dirname(HERE)
CORE = os.path.join(os.path.dirname(ANDROID), "core")
APP_LICENCE = os.path.join(os.path.dirname(ANDROID), "LICENSE")
APP_URL = "https://github.com/rogertobler/missionctrl"
TEXTS = os.path.join(HERE, "license-texts")
OUT = os.path.join(ANDROID, "app/src/main/assets/licenses/third_party.json")
FONT_LICENCE = os.path.join(ANDROID, "app/src/main/assets/licenses/IBM-Plex-OFL.txt")

texts = {}


def text_id(text):
    text = text.replace("\r\n", "\n").strip() + "\n"
    key = hashlib.sha1(text.encode()).hexdigest()[:12]
    texts.setdefault(key, text)
    return key


def standard(spdx):
    path = os.path.join(TEXTS, spdx + ".txt")
    return text_id(open(path, encoding="utf-8").read()) if os.path.exists(path) else None


def licence_files(folder):
    found = []
    for root, dirs, files in os.walk(folder):
        dirs[:] = sorted(d for d in dirs if not d.startswith(".") and d != "target")
        if root[len(folder):].count(os.sep) > 1:
            dirs[:] = []
        for f in sorted(files):
            if f.upper().startswith(("LICENSE", "LICENCE", "COPYING", "UNLICENSE", "NOTICE")):
                found.append(os.path.join(root, f))
    return found


def cargo_metadata():
    return json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", "aarch64-linux-android"], cwd=CORE))


def app(meta):
    ffi = next(p for p in meta["packages"] if p["name"] == "mc-ffi")
    return [{"name": "MissionCtrl", "version": ffi["version"], "license": ffi["license"], "url": APP_URL,
             "texts": [text_id(open(APP_LICENCE, encoding="utf-8").read())]}]


def rust(meta):
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = next(p["id"] for p in meta["packages"] if p["name"] == "mc-ffi")
    seen, stack = set(), [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        stack += [d["pkg"] for d in nodes[pid]["deps"] if any(k["kind"] is None for k in d["dep_kinds"])]
    items = []
    for pid in seen:
        p = packages[pid]
        if p["source"] is None:  # our own crates
            continue
        licence = p.get("license") or "see files"
        ids = [text_id(open(f, encoding="utf-8", errors="replace").read()) for f in licence_files(os.path.dirname(p["manifest_path"]))]
        if not ids:
            for spdx in re.findall(r"[A-Za-z0-9.+-]+", licence):
                key = standard(spdx)
                if key:
                    ids = [key]
                    break
        items.append({"name": p["name"], "version": p["version"], "license": licence,
                      "url": p.get("repository") or p.get("homepage") or "", "texts": sorted(set(ids))})
    return sorted(items, key=lambda i: i["name"].lower())


def android():
    out = subprocess.check_output(
        ["./gradlew", "-q", ":app:dependencies", "--configuration", "releaseRuntimeClasspath"], cwd=ANDROID, text=True)
    found = {}
    for m in re.finditer(r"([a-z0-9_.-]+):([A-Za-z0-9_.-]+)(?::([0-9][\w.+-]*))?(?: -> ([0-9][\w.+-]*))?", out):
        group, name, version = m.group(1), m.group(2), m.group(4) or m.group(3)
        if version:
            found[f"{group}:{name}"] = version
    apache = standard("Apache-2.0")
    items = []
    for coord, version in found.items():
        group = coord.split(":")[0]
        if group == "net.java.dev.jna":
            licence, ids, url = "Apache-2.0 OR LGPL-2.1-or-later", [apache, standard("LGPL-2.1-or-later")], "https://github.com/java-native-access/jna"
        elif group.startswith("androidx."):
            licence, ids, url = "Apache-2.0", [apache], "https://developer.android.com/jetpack/androidx"
        elif group.startswith(("org.jetbrains.kotlin", "org.jetbrains")):
            licence, ids, url = "Apache-2.0", [apache], "https://kotlinlang.org"
        else:
            licence, ids, url = "Apache-2.0", [apache], ""
        items.append({"name": coord, "version": version, "license": licence, "url": url, "texts": ids})
    return sorted(items, key=lambda i: i["name"])


def main():
    meta = cargo_metadata()
    groups = [
        {"key": "app", "items": app(meta)},
        {"key": "android", "items": android()},
        {"key": "core", "items": rust(meta)},
        {"key": "fonts", "items": [{"name": "IBM Plex Sans, IBM Plex Mono", "version": "", "license": "OFL-1.1",
                                    "url": "https://github.com/IBM/plex", "texts": [text_id(open(FONT_LICENCE, encoding="utf-8").read())]}]},
        {"key": "models", "items": [{"name": "Whisper (OpenAI), ggml models", "version": "", "license": "MIT",
                                     "url": "https://github.com/openai/whisper", "texts": [standard("whisper-models")]}]},
    ]
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        json.dump({"groups": groups, "texts": texts}, f, ensure_ascii=False, separators=(",", ":"))
    print(f"{OUT}: {sum(len(g['items']) for g in groups)} entries, {len(texts)} texts, {os.path.getsize(OUT) // 1024} KB")


if __name__ == "__main__":
    main()
