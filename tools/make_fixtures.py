"""Writes crates/cbs-core/tests/fixtures/python.json: outputs of the Python version's
functions for generated inputs. The Rust tests check that the port gives the same.

    python3 tools/make_fixtures.py /path/to/chatbox-studio/src
"""
import ast
import json
import os
import random
import sys

src = sys.argv[1]
sys.path.insert(0, src)
import chatbox as C  # noqa: E402
import filters as F  # noqa: E402

# split_for_chatbox lives in engine.py, which needs the speech libraries; take just it.
tree = ast.parse(open(os.path.join(src, "engine.py"), encoding="utf-8").read())
fn = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "split_for_chatbox")
ns = {"re": __import__("re"), "CHATBOX_LIMIT": 144}
exec(compile(ast.Module([fn], []), "engine.py", "exec"), ns)
split_for_chatbox = ns["split_for_chatbox"]

rnd = random.Random(1234)
WORDS = ["hello", "world", "a", "VRChat", "über", "naïve", "😀", "🎵", "日本語", "x" * 30,
         "um,", "uh", "so", "colour", "Colour", "5", "euros", "the", "I", "think", "—", "...",
         "verylongwordwithoutanyspacesatallthatgoesonandon"]


def text():
    parts = []
    for _ in range(rnd.randint(0, 24)):
        parts.append(rnd.choice(WORDS))
        parts.append(rnd.choice([" ", " ", " ", "  ", "\n", "\t", ", "]))
    return "".join(parts)


cases = []
for _ in range(60):
    t = text()
    w = rnd.choice([5, 10, 20, 40, 72, 144])
    step = rnd.choice([0, 1, 2, 3, 5])
    lines = rnd.choice([1, 2, 3])
    cases.append({
        "text": t, "width": w, "step": step, "lines": lines,
        "wrap": C.wrap_words(t, w),
        "scroll": C.scroll_windows(t, w, max(1, step)),
        "ticker": C.ticker_frames(t, w, max(1, step)),
        "marquee": C.marquee_frames(t, w, max(1, step)),
        "bounce": C.bounce_frames(t, w, max(1, step)),
        "lines_f": C.line_frames(t, w, lines),
        "typewriter": C.typewriter_frames(t, w, max(1, step)),
        "truncate": C.truncate(t, w),
        "truncate16": C.truncate16(t, w),
        "split": split_for_chatbox(t, w),
    })

templates = ["🎵 {title}[ — {artist}]", "[EN] {x}", "{a}{b}[{c}]\\n{d}", "[ {a} and {b} ]",
             "plain", "{unknown} {a}"]
fills = []
for _ in range(80):
    vals = {k: rnd.choice(["", " ", "v", "Wert", "😀"]) for k in "abcdx" if rnd.random() < 0.8}
    vals.update({k: rnd.choice(["", "Song", "Band"]) for k in ("title", "artist")
                 if rnd.random() < 0.8})
    tpl = rnd.choice(templates)
    fills.append({"template": tpl, "values": vals, "out": C.fill(tpl, vals)})

durations = [{"s": s, "sec": ws, "out": C.fmt_duration(s, ws)}
             for s in [0, 1, 59, 60, 61, 3599, 3600, 3661, 86399, 86400, 90061, 1e7, 12.7]
             for ws in (False, True)]


class S:
    filters_enabled = True
    remove_fillers = True
    min_words = 0
    replace_rules = [{"find": "vr chat", "replace": "VRChat"},
                     {"find": "colour", "replace": "color"},
                     {"find": r"(\d+) euros?", "replace": r"€\1", "regex": True,
                      "whole_word": False}]
    block_rules = [{"pattern": "naïve"}]


fs = F.FilterSet.from_settings(S)
filtered = []
for _ in range(150):
    t = text()
    o = fs.apply(t)
    filtered.append({"text": t, "out": o.text, "blocked": o.blocked, "reason": o.reason})

out = {"frames": cases, "fill": fills, "durations": durations, "filters": filtered}
path = os.path.join(os.path.dirname(__file__), "..", "crates", "cbs-core", "tests", "fixtures",
                    "python.json")
with open(path, "w", encoding="utf-8") as f:
    json.dump(out, f, ensure_ascii=False, separators=(",", ":"))
print("wrote", os.path.normpath(path))
