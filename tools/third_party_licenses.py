#!/usr/bin/env python3
"""Writes THIRD_PARTY_LICENSES.md: the license notice of every Rust crate texcopy is built from.

    python3 tools/third_party_licenses.py THIRD_PARTY_LICENSES.md   # or any other path

The file is not kept in the repository: the packaging scripts generate it into each download, so it
always matches Cargo.lock. (A copy written to the repo root is ignored by git.)

MIT, Apache-2.0, BSD, ISC, OFL and MPL all require their copyright and license notices to travel
with a distributed binary, so every download ships this file (the packaging scripts run this tool).

It reads the crates already on disk (after a build they are in ~/.cargo/registry), so it needs no
network. Crates are the ones the program is built from on any platform (normal and build
dependencies for macOS, Windows and Linux; test-only ones are left out). Identical license texts are printed once, followed by the
crates that use them.
"""
import collections
import glob
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# not Rust crates but they ship in or get fetched by texcopy, kept by hand.
NOT_CRATES = """\
## Not Rust crates

- **ONNX Runtime** (MIT, Microsoft, <https://github.com/microsoft/onnxruntime>) is statically linked
  into the program through the `ort` crate. Its license is reproduced below. The third-party
  components inside ONNX Runtime (protobuf, abseil, onnx, flatbuffers, re2 and others) have their own
  notices in the `ThirdPartyNotices.txt` of the ONNX Runtime source tree.
- **Fonts** inside the window come from egui's `epaint_default_fonts` (listed below with their licenses).
- **Models and helper, downloaded on first use and not part of this download:**
  - `pix2text-mfr` by breezedeus (MIT), the small formula model: <https://huggingface.co/breezedeus/pix2text-mfr>
  - OvisOCR2 (Apache-2.0), the vision model, GGUF conversion by bartowski: <https://huggingface.co/bartowski/ATH-MaaS_OvisOCR2-GGUF>
  - `llama-server` from llama.cpp (MIT, ggml-org): <https://github.com/ggml-org/llama.cpp>
"""

# crates whose package has no license file, print the standard text once with the holder from the crate's metadata.
STANDARD_MIT = """\
Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT
OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
"""
STANDARD_BSD2 = """\
Redistribution and use in source and binary forms, with or without modification, are permitted
provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions
   and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice, this list of
   conditions and the following disclaimer in the documentation and/or other materials provided
   with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER
IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF
THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
"""
STANDARD_BSL1 = """\
Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization obtaining a copy of the
software and accompanying documentation covered by this license (the "Software") to use, reproduce,
display, distribute, execute, and transmit the Software, and to prepare derivative works of the
Software, and to permit third-parties to whom the Software is furnished to do so, all subject to the
following:

The copyright notices in the Software and this entire statement, including the above license grant,
this restriction and the following disclaimer, must be included in all copies of the Software, in
whole or in part, and all derivative works of the Software, unless such copies or derivative works
are solely in the form of machine-executable object code generated by a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, TITLE AND
NON-INFRINGEMENT. IN NO EVENT SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE
LIABLE FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT
OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
"""

ONNX_RUNTIME_LICENSE = """\
MIT License

Copyright (c) Microsoft Corporation

""" + STANDARD_MIT

LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|unlicense|notice|copyright)([-_. ].*)?$", re.I)


def run(*cmd):
    # cargo prints UTF-8 and windows would decode it as cp1252 without this.
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf8", errors="replace", check=True).stdout


# the platforms texcopy is released for.
TARGETS = ("aarch64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu")


def shipped_crates():
    """(name, version) of every crate the program is built from on a platform we release for."""
    out = set()
    for target in TARGETS:
        tree = run("cargo", "tree", "--locked", "--target", target, "-e", "normal,build",
                   "--prefix", "none", "--format", "{p}")
        for line in tree.splitlines():
            m = re.match(r"(\S+) v(\S+)", line.strip())
            if m:
                out.add((m.group(1), m.group(2)))
    return out


def license_texts(crate_dir):
    """The license files at the top of a crate (and in a licenses/ folder), as {filename: text}."""
    found = {}
    # egui's default fonts keep their license texts next to the font files.
    for pattern in ("*", os.path.join("licenses", "*"), os.path.join("LICENSES", "*"), os.path.join("fonts", "*.txt")):
        for path in sorted(glob.glob(os.path.join(crate_dir, pattern))):
            name = os.path.basename(path)
            in_fonts = os.path.basename(os.path.dirname(path)) == "fonts"
            if os.path.isfile(path) and (in_fonts or LICENSE_FILE.match(name)) and not name.lower().endswith((".rs", ".toml")):
                try:
                    found[os.path.relpath(path, crate_dir)] = open(path, encoding="utf8", errors="replace").read().strip()
                except OSError:
                    pass
    return found


def main():
    wanted = shipped_crates()
    meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--locked"))
    by_text = collections.defaultdict(list) # text hash -> [(crate, version, spdx, repo, filename)]
    texts = {}
    without = []
    for pkg in sorted(meta["packages"], key=lambda p: (p["name"], p["version"])):
        key = (pkg["name"], pkg["version"])
        if key not in wanted or pkg["name"] == "texcopy":
            continue
        crate_dir = os.path.dirname(pkg["manifest_path"])
        files = license_texts(crate_dir)
        spdx = pkg.get("license") or "(see repository)"
        if not files:
            without.append((pkg["name"], pkg["version"], spdx, pkg.get("repository") or "", pkg.get("authors") or []))
            continue
        for fname, text in files.items():
            h = hashlib.sha256(re.sub(r"\s+", " ", text).encode()).hexdigest()
            texts[h] = text
            by_text[h].append((pkg["name"], pkg["version"], spdx, pkg.get("repository") or "", fname))

    out = []
    out.append("# Third-party licenses\n")
    out.append("texcopy is MIT licensed (see `LICENSE`). It is built from the open-source software below, "
               "and this file reproduces the license notices those licenses require. "
               f"Generated by `tools/third_party_licenses.py` from {len(wanted) - 1} crates.\n")
    out.append(NOT_CRATES)
    out.append("### ONNX Runtime: MIT\n")
    out.append("```text\n" + ONNX_RUNTIME_LICENSE.strip() + "\n```\n")
    out.append("## Rust crates\n")
    out.append("Each license text is printed once, followed by the crates that carry it. "
               "Where a crate offers a choice of licenses (`MIT OR Apache-2.0`), texcopy uses it under the permissive one.\n")
    ordered = sorted(by_text.items(), key=lambda kv: (-len({c[:2] for c in kv[1]}), kv[0]))
    for h, users in ordered:
        crates = sorted({(n, v, s) for n, v, s, _, _ in users})
        spdx = sorted({s for _, _, s in crates})
        out.append(f"### {' / '.join(spdx)}\n")
        out.append("Used by: " + ", ".join(f"`{n} {v}`" for n, v, _ in crates) + "\n")
        out.append("```text\n" + texts[h] + "\n```\n")
    if without:
        out.append("## Crates whose published package has no license file\n")
        out.append("These crates declare their license in their `Cargo.toml` but the text sits at the root of "
                   "their source repository, outside the published package. They are listed with their copyright "
                   "holders, and are used under the permissive license named: the standard texts follow.\n")
        for n, v, s_, repo, authors in without:
            holders = ", ".join(a.split("<")[0].strip() for a in authors) or "the authors (see the repository)"
            out.append(f"- `{n} {v}`: {s_}. Copyright: {holders}." + (f" <{repo}>" if repo else ""))
        out.append("")
        used = " ".join(x[2] for x in without)
        if "MIT" in used:
            out.append("### Standard text: MIT\n\nCopyright (c) the holders listed above.\n")
            out.append("```text\n" + STANDARD_MIT.strip() + "\n```\n")
        if "Apache-2.0" in used:
            apache = next((t for t in texts.values() if t.lstrip().startswith("Apache License") and "Version 2.0" in t[:200]), None)
            out.append("### Standard text: Apache-2.0\n")
            out.append(("```text\n" + apache + "\n```\n") if apache else "See <https://www.apache.org/licenses/LICENSE-2.0>.\n")
        if "BSD-2-Clause" in used:
            out.append("### Standard text: BSD-2-Clause\n\nCopyright (c) the holders listed above.\n")
            out.append("```text\n" + STANDARD_BSD2.strip() + "\n```\n")
        if "BSL-1.0" in used:
            out.append("### Standard text: BSL-1.0\n")
            out.append("```text\n" + STANDARD_BSL1.strip() + "\n```\n")
    text = "\n".join(out) + "\n"
    if len(sys.argv) > 1:
        with open(sys.argv[1], "w", encoding="utf8", newline="\n") as f: # UTF-8 on every OS.
            f.write(text)
    else:
        sys.stdout.write(text)
    print(f"{len(wanted) - 1} crates, {len(by_text)} distinct license texts, {len(without)} without a file",
          file=sys.stderr)


if __name__ == "__main__":
    main()
