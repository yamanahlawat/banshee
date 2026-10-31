#!/usr/bin/env python3
"""Writes banshee-common/src/listening/catalogue.rs from the transcribe.cpp
models on Hugging Face. Run it by hand with `python3 -I`, then review and
commit the diff; the build never runs it and never touches the network.

Standard library only, plus rustfmt on the PATH. Any fact it cannot read from a source stops the run.
"""

import hashlib
import http.client
import json
import re
import struct
import subprocess
import sys
import urllib.request
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parent.parent
LISTENING = ROOT / "banshee-common/src/listening.rs"
OUTPUT = ROOT / "banshee-common/src/listening/catalogue.rs"

API = "https://huggingface.co/api"
AUTHOR = "handy-computer"
LIST_URL = f"{API}/models?author={AUTHOR}&limit=1000"

NOT_SPEECH = {
    "silero-vad-v6.2-gguf",
    "lang-id-voxlingua107-ecapa-gguf",
    "diar_streaming_sortformer_4spk-v2.1-gguf",
}


class Licence(NamedTuple):
    const: str
    # With its version or date, so a revised text needs a fresh acceptance.
    name: str
    # None takes the card's license_link, which every repo under it must share.
    url: str | None
    summary: str
    # The person sees the terms and accepts them before the file downloads.
    needs_acceptance: bool
    # The attribution the licence asks to show wherever the model is offered.
    credit: str | None = None


LICENCES = {
    "mit": Licence(
        "MIT",
        "MIT",
        "https://spdx.org/licenses/MIT.html",
        "Use, copy, change and share it, keeping the copyright notice.",
        False,
    ),
    "apache-2.0": Licence(
        "APACHE_2_0",
        "Apache-2.0",
        "https://www.apache.org/licenses/LICENSE-2.0",
        "Use, change and share it, keeping its notices.",
        False,
    ),
    "cc-by-4.0": Licence(
        "CC_BY_4_0",
        "CC-BY-4.0",
        "https://creativecommons.org/licenses/by/4.0/",
        "Use and share it for any purpose, crediting its authors.",
        False,
    ),
    "cc-by-sa-4.0": Licence(
        "CC_BY_SA_4_0",
        "CC-BY-SA-4.0",
        "https://creativecommons.org/licenses/by-sa/4.0/",
        "Use and share it for any purpose, crediting its authors; a changed copy you share keeps this licence.",
        False,
    ),
    "cc-by-nc-4.0": Licence(
        "CC_BY_NC_4_0",
        "CC-BY-NC-4.0",
        "https://creativecommons.org/licenses/by-nc/4.0/",
        "Use and share it, crediting its authors, for non-commercial purposes only.",
        True,
    ),
    "cc-by-nc-sa-4.0": Licence(
        "CC_BY_NC_SA_4_0",
        "CC-BY-NC-SA-4.0",
        "https://creativecommons.org/licenses/by-nc-sa/4.0/",
        "Use and share it, crediting its authors, for non-commercial purposes only; a changed copy you share keeps this licence or a compatible one.",
        True,
    ),
}

# A card that says `other` is known only by the sha256 of the LICENSE file its
# repo ships, each one read by hand. A changed text is unknown.
OTHER_LICENCES = {
    "99d771ff4869fcddca4907a360e04d4c0a4627dcdc0d74f6618acabe64da5e75": Licence(
        "NVIDIA_OPEN_MODEL",
        "NVIDIA Open Model License (2025-10-24)",
        None,
        "Commercial use is allowed; use must follow NVIDIA's Trustworthy AI terms, and the licence ends if its guardrails are bypassed.",
        False,
    ),
    "7dba975a2069691db4992b0592d70828b330d2f8a30a71450f4e152a554e84f8": Licence(
        "FUNASR_1_1",
        "FunASR Model License 1.1",
        None,
        "Free to use, copy, change and share, crediting the source and author and keeping the model's name; it says the model is for reference and learning only, the licence ends for unjustified denigration, malicious smearing or baseless insults of FunASR, and the terms may be revised.",
        True,
    ),
    "9b9434e5a78640ca71262192cb6e5730c13371f350cdf0643555a512e7badca5": Licence(
        "HAI_DEF",
        "Health AI Developer Foundations Terms of Use (2024-11-15)",
        None,
        "Use must follow Google's Prohibited Use Policy and the law, and must not lead a health regulator to deem Google a medical device manufacturer; you indemnify Google, and Google may end the terms if you breach them or if Google could be deemed a device manufacturer.",
        True,
    ),
    "2ab44b68365473c112f5092211a38f231cb23e50de68b75a13369adbd76a74df": Licence(
        "OPENMDW_1_1",
        "OpenMDW-1.1",
        None,
        "Use, change and share it without restriction, keeping the licence and its notices.",
        False,
    ),
    "af5b7213e07135d1dcd2c745612fa6b074b1f982879f6d52158062f9245b5964": Licence(
        "MOONSHINE_COMMUNITY",
        "Moonshine AI Community License (2025-06-15)",
        None,
        "Free for research and non-commercial use; commercial use needs registration with Moonshine AI, and the licence ends once you or your affiliates have more than USD 1M annual revenue.",
        True,
        "Powered by Moonshine AI",
    ),
}
NVIDIA_NOTICE = "Licensed by NVIDIA Corporation under the NVIDIA Open Model License"

# Whether a family weighs the words to know, from transcribe-cpp-sys 0.4.1:
# src/arch/*/capabilities.cpp, and model.cpp for the two that decide per file.
TAKES_VOCABULARY = {
    "whisper": True,
    "funasr_nano": True,
    "qwen3_asr": True,
    "canary": False,
    "canary_qwen": False,
    "cohere_asr": False,
    "gigaam": False,
    "granite_speech_nar": False,
    "granite_speech5_ctc": False,
    "medasr": False,
    "moonshine": False,
    "moonshine_streaming": False,
    "parakeet": False,
    "sensevoice": False,
    "voxtral": False,
    "voxtral_realtime": False,
    "granite_speech": "file",
    "moss": "file",
}


class Refused(Exception):
    pass


def get(url, headers=None, attempts=3):
    request = urllib.request.Request(url, headers=headers or {})
    for attempt in range(attempts):
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                asked = request.headers.get("Range")
                if asked is not None:
                    start = asked.removeprefix("bytes=").split("-")[0]
                    answered = response.headers.get("Content-Range", "")
                    if response.status != 206 or not answered.startswith(f"bytes {start}-"):
                        raise Refused(f"{url} answered {asked} with {response.status} {answered!r}")
                return response.read()
        except http.client.IncompleteRead:
            if attempt == attempts - 1:
                raise


def get_json(url):
    return json.loads(get(url))


def resolve_url(repo, revision, path):
    return f"https://huggingface.co/{repo}/resolve/{revision}/{path}"


class RangeReader:
    """Reads the head of a remote file a few megabytes at a time."""

    STEP = 2 * 1024 * 1024

    def __init__(self, url):
        self.url = url
        self.data = b""
        self.offset = 0

    def take(self, count):
        while len(self.data) < self.offset + count:
            start = len(self.data)
            end = start + self.STEP - 1
            chunk = get(self.url, {"Range": f"bytes={start}-{end}"})
            if not chunk:
                raise Refused(f"{self.url} ended inside its metadata")
            self.data += chunk
        piece = self.data[self.offset : self.offset + count]
        self.offset += count
        return piece

    def unpack(self, fmt):
        return struct.unpack("<" + fmt, self.take(struct.calcsize("<" + fmt)))[0]


SCALARS = {0: "B", 1: "b", 2: "H", 3: "h", 4: "I", 5: "i", 6: "f", 7: "?", 10: "Q", 11: "q", 12: "d"}


def gguf_value(reader, kind):
    if kind in SCALARS:
        return reader.unpack(SCALARS[kind])
    if kind == 8:
        return reader.take(reader.unpack("Q")).decode("utf-8", "replace")
    if kind == 9:
        inner = reader.unpack("I")
        return [gguf_value(reader, inner) for _ in range(reader.unpack("Q"))]
    raise Refused(f"unknown GGUF value type {kind}")


def gguf_metadata(url):
    reader = RangeReader(url)
    if reader.take(4) != b"GGUF":
        raise Refused(f"{url} is not a GGUF file")
    reader.unpack("I")
    reader.unpack("Q")
    metadata = {}
    for _ in range(reader.unpack("Q")):
        key = reader.take(reader.unpack("Q")).decode()
        metadata[key] = gguf_value(reader, reader.unpack("I"))
    return metadata


def byte_level_decoder():
    """GPT-2's byte-to-character table, inverted, as byte-level BPE tokenizers use it."""
    printable = list(range(ord("!"), ord("~") + 1)) + list(range(ord("\xa1"), ord("\xac") + 1)) + list(range(ord("\xae"), ord("\xff") + 1))
    chars = printable[:]
    extra = 0
    for byte in range(256):
        if byte not in printable:
            printable.append(byte)
            chars.append(256 + extra)
            extra += 1
    return {chr(char): byte for byte, char in zip(printable, chars)}


def moss_takes_vocabulary(metadata):
    """moss_supports_hotwords in arch/moss/model.cpp: an instruction split, a
    tokenizer that can encode (Tokenizer::has_encoder: gpt2 with merges), and a
    suffix equal to the head tokens, the encoded instruction and the tail tokens.
    This checks the framing exactly and the middle by decoding it, since encoding
    needs the BPE pre-tokenizer this script does not run."""
    instruction = metadata.get("stt.moss.prompt_instruction", "")
    head = metadata.get("stt.moss.prompt_instruction_head_tokens")
    tail = metadata.get("stt.moss.prompt_instruction_tail_tokens")
    if not instruction or head is None or tail is None:
        return False
    if metadata.get("tokenizer.ggml.model") != "gpt2" or not metadata.get("tokenizer.ggml.merges"):
        return False
    suffix = metadata["stt.moss.prompt_suffix_tokens"]
    if suffix[: len(head)] != head or suffix[len(suffix) - len(tail) :] != tail:
        raise Refused("the moss prompt suffix is not framed by the instruction's head and tail")
    tokens = metadata["tokenizer.ggml.tokens"]
    decoder = byte_level_decoder()
    middle = suffix[len(head) : len(suffix) - len(tail)]
    decoded = bytes(decoder[char] for token in middle for char in tokens[token]).decode("utf-8")
    if decoded != instruction:
        raise Refused("the moss prompt suffix does not hold the instruction")
    return True


def vocabulary_from_file(family, metadata):
    if family == "granite_speech":
        # Absent means unsupported, as arch/granite/model.cpp reads it.
        return bool(metadata.get("stt.capability.vocabulary", False))
    return moss_takes_vocabulary(metadata)


def licence_of(repo, revision, card, files):
    licence = card.get("license")
    if licence in LICENCES:
        return LICENCES[licence]
    if licence != "other":
        raise Refused(f"licence {licence!r} is not one Banshee knows")
    if "LICENSE" not in files:
        raise Refused("licence 'other' with no LICENSE file")
    digest = hashlib.sha256(get(resolve_url(repo, revision, "LICENSE"))).hexdigest()
    if digest not in OTHER_LICENCES:
        raise Refused(f"licence 'other' ({card.get('license_name')}) with an unread LICENSE, sha256 {digest}")
    known = OTHER_LICENCES[digest]
    if known.const == "NVIDIA_OPEN_MODEL":
        if "NOTICE" not in files or NVIDIA_NOTICE not in get(resolve_url(repo, revision, "NOTICE")).decode():
            raise Refused("NVIDIA Open Model License with no NOTICE naming it")
    link = required(card, "license_link", str, "the card")
    return known._replace(url=link)


def required(mapping, key, kind, where):
    value = mapping.get(key)
    if not isinstance(value, kind):
        raise Refused(f"{where} lacks {key}")
    return value


def entry_for(repo, info, revision, file_name=None):
    card = info.get("cardData") or {}
    tree = get_json(f"{API}/models/{repo}/tree/{revision}")
    files = {item["path"]: item for item in tree}
    if file_name is None:
        quantised = [path for path in files if path.endswith("-Q8_0.gguf")]
        if len(quantised) != 1:
            raise Refused(f"{len(quantised)} Q8_0 files")
        file_name = quantised[0]
    lfs = files.get(file_name, {}).get("lfs")
    if not lfs:
        raise Refused(f"{file_name} has no LFS record")

    licence = licence_of(repo, revision, card, files)
    languages = required(card, "language", list, "the card")
    if not languages or not all(isinstance(code, str) for code in languages):
        raise Refused("the card's language list is empty or holds a value that is not a code")
    facts = required(card, "transcribe_cpp", dict, "the card")
    family = required(info.get("gguf") or {}, "architecture", str, "the GGUF summary")
    if family not in TAKES_VOCABULARY:
        raise Refused(f"family {family!r} is not one transcribe-cpp-sys 0.4.1 names")
    takes_vocabulary = TAKES_VOCABULARY[family]
    if takes_vocabulary == "file":
        takes_vocabulary = vocabulary_from_file(family, gguf_metadata(resolve_url(repo, revision, file_name)))

    return {
        "id": repo.split("/", 1)[1].removesuffix("-gguf"),
        "repo": repo,
        "revision": revision,
        "file": file_name,
        "bytes": lfs["size"],
        "sha256": lfs["oid"],
        "licence": licence,
        "languages": languages,
        "translates": required(facts, "translate", bool, "the card's transcribe_cpp"),
        "streams": required(facts, "streaming", bool, "the card's transcribe_cpp"),
        "takes_vocabulary": takes_vocabulary,
    }


def pinned_suggestions():
    """The hand-checked entries in listening.rs, which this script never writes."""
    source = LISTENING.read_text()
    pins = []
    for block in re.findall(r"ListeningModel = ListeningModel \{(.*?)\n\};", source, re.S):
        fields = dict(re.findall(r"(\w+): ([^,\n\[]+),", block))
        languages = re.search(r"languages: &\[(.*?)\],", block, re.S)
        if languages:
            fields["languages"] = rust_value(re.findall(r'"([^"]*)"', languages.group(1)))
        pins.append(fields)
    if len(pins) != 2:
        raise Refused(f"listening.rs holds {len(pins)} pinned entries, expected 2")
    return pins


def rust_value(value):
    if value is None:
        return "None"
    if isinstance(value, Licence):
        return f"&{value.const}"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return f"{value:_}"
    if isinstance(value, list):
        return "&[" + ", ".join(json.dumps(item) for item in value) + "]"
    return json.dumps(value)


def check_pin(pin, entry):
    for key, value in entry.items():
        if key == "id":
            continue
        written = pin.get(key, "").strip().replace("&catalogue::", "&")
        if written != rust_value(value):
            raise Refused(f"the pin says {key} = {written}, upstream says {rust_value(value)}")


def render(entries, licences):
    lines = [
        "// Generated by scripts/listening-catalogue.py from",
        f"// {LIST_URL}.",
        "// Do not edit; run the script and review the diff.",
        "",
        "use super::{Licence, ListeningModel, PARAKEET, TURBO};",
        "",
    ]
    for licence in sorted(licences.values()):
        lines.append(f"pub const {licence.const}: Licence = Licence {{")
        for key in ("name", "url", "summary", "needs_acceptance", "credit"):
            value = getattr(licence, key)
            rendered = f"Some({rust_value(value)})" if key == "credit" and value is not None else rust_value(value)
            lines.append(f"    {key}: {rendered},")
        lines.append("};")
        lines.append("")
    lines += [
        "pub static CATALOGUE: &[ListeningModel] = &[",
        "    PARAKEET,",
        "    TURBO,",
    ]
    for entry in entries:
        lines.append("    ListeningModel {")
        for key, value in entry.items():
            lines.append(f"        {key}: {rust_value(value)},")
            if key == "id":
                lines.append("        label: None,")
        lines.append("    },")
    lines.append("];")
    return "\n".join(lines) + "\n"


def collect(licences, licence):
    """One definition per licence: every repo under it must agree on its link."""
    seen = licences.setdefault(licence.const, licence)
    if seen != licence:
        raise Refused(f"{licence.const} links to {licence.url}, another repo to {seen.url}")


def main():
    pins = {pin["repo"].strip('"'): pin for pin in pinned_suggestions()}
    repos = sorted(model["id"] for model in get_json(LIST_URL))
    kept, excluded, licences = [], [], {}
    for repo in repos:
        name = repo.split("/", 1)[1]
        if name in NOT_SPEECH:
            excluded.append((name, "not speech-to-text"))
            continue
        info = get_json(f"{API}/models/{repo}")
        try:
            if info.get("gated"):
                raise Refused(f"gated ({info['gated']}): it needs a Hugging Face login")
            if repo in pins:
                pin = pins[repo]
                entry = entry_for(repo, info, pin["revision"].strip('"'), pin["file"].strip('"'))
                check_pin(pin, entry)
                collect(licences, entry["licence"])
                print(f"pinned  {entry['id']}: matches upstream at {entry['revision']}")
                continue
            entry = entry_for(repo, info, info["sha"])
            collect(licences, entry["licence"])
        except Refused as reason:
            print(f"error   {name}: {reason}", file=sys.stderr)
            sys.exit(1)
        kept.append(entry)
        print(f"kept    {entry['id']}: {entry['file']}, {entry['licence'].name}")
    ids = [entry["id"].lower() for entry in kept] + [repo.split("/", 1)[1].removesuffix("-gguf").lower() for repo in pins]
    if len(set(ids)) != len(ids):
        print("error   two ids differ only by case, and Banshee matches ids ignoring case", file=sys.stderr)
        sys.exit(1)
    OUTPUT.parent.mkdir(exist_ok=True)
    OUTPUT.write_text(render(kept, licences))
    # The language lists run past rustfmt's width, so its layout is the one committed.
    subprocess.run(["rustfmt", "--edition", "2024", str(OUTPUT)], check=True)
    for name, reason in excluded:
        print(f"skipped {name}: {reason}")
    print(f"{len(kept) + len(pins)} entries, {len(pins)} of them pinned by hand; wrote {OUTPUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
