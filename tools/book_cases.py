#!/usr/bin/env python3
"""Cut real exercise screenshots out of a textbook PDF, with ground truth from its own text layer.

  python3 tools/book_cases.py book.pdf --out tools/fixtures/private --n 40

Works on PDFs that have a real text layer and italic variables (typically ebook conversions). The
crops are copyrighted material: keep them out of git (tools/fixtures/private/ is gitignored).

Truth is built from pdftohtml's span data: italic spans (and small raised/lowered spans such as
subscripts) are math and get wrapped in $...$; everything else is text.
"""
import html, json, os, random, re, subprocess, sys

BOOK = OUT = None
DPI = 200
SYM = {"≤": r"\le ", "≥": r"\ge ", "∈": r"\in ", "∉": r"\notin ", "∪": r"\cup ", "∩": r"\cap ", "⊆": r"\subseteq ",
       "∅": r"\emptyset ", "×": r"\times ", "→": r"\to ", "≠": r"\ne ", "−": "-", "–": "-"}


def page_xml(p):
    x = subprocess.run(["pdftohtml", "-xml", "-f", str(p), "-l", str(p), "-stdout", BOOK], capture_output=True, text=True).stdout
    w, h = map(float, re.search(r'<page number="\d+" position="absolute" top="0" left="0" height="(\d+)" width="(\d+)"', x).groups()[::-1])
    fonts = {m.group(1): int(m.group(2)) for m in re.finditer(r'<fontspec id="(\d+)" size="(\d+)"', x)}
    spans = []
    for m in re.finditer(r'<text top="(\d+)" left="(\d+)" width="(\d+)" height="(\d+)" font="(\d+)">(.*?)</text>', x):
        top, left, width, height, font, inner = m.groups()
        italic = "<i>" in inner
        text = html.unescape(re.sub(r"</?[a-z][^>]*>", "", inner))
        spans.append(dict(top=int(top), left=int(left), width=int(width), height=int(height), size=fonts[font], italic=italic, text=text))
    images = [tuple(map(int, m.groups())) for m in re.finditer(r'<image top="(\d+)" left="(\d+)" width="(\d+)" height="(\d+)"', x)]
    return w, h, spans, images


def lines_of(spans):
    if not spans:
        return []
    body = max(set(s["size"] for s in spans), key=lambda z: sum(1 for s in spans if s["size"] == z)) # most common size.
    main = sorted([s for s in spans if s["size"] >= body - 3], key=lambda s: (s["top"], s["left"]))
    small = [s for s in spans if s["size"] < body - 3]
    lines = []
    for s in main:
        base = s["top"] + s["height"]
        for ln in lines:
            if abs(ln["base"] - base) <= 6:
                ln["spans"].append(s); break
        else:
            lines.append(dict(base=base, size=s["size"], spans=[s]))
    for s in small: # sub and superscripts belong to the line they sit next to.
        mid = s["top"] + s["height"] / 2
        best = min(lines, key=lambda ln: abs(mid - (ln["base"] - body / 2)), default=None)
        if best is not None and abs(mid - (best["base"] - body / 2)) <= body:
            best["spans"].append(s)
    for ln in lines:
        ln["spans"].sort(key=lambda s: s["left"])
        ln["top"] = min(s["top"] for s in ln["spans"]); ln["bottom"] = max(s["top"] + s["height"] for s in ln["spans"])
        ln["left"] = ln["spans"][0]["left"]; ln["right"] = max(s["left"] + s["width"] for s in ln["spans"])
        ln["body"] = body
    return sorted(lines, key=lambda l: l["top"])


def prev_ok(out, run):
    return bool(run) and run[-1] in "=<>+-*/" or run.endswith(tuple(SYM.values()))


def line_truth(ln):
    """Text of one line with math runs wrapped in $...$."""
    body = ln["body"]
    parts = [] # (kind, text, left, right)
    for s in ln["spans"]:
        t = s["text"]
        script = s["size"] < body - 3
        if script:
            up = (s["top"] + s["height"] / 2) < (ln["base"] - body * 0.55)
            parts.append(("script", ("^{%s}" if up else "_{%s}") % t.strip(), s["left"], s["left"] + s["width"]))
        elif s["italic"] and t.strip():
            parts.append(("math", t.strip(), s["left"], s["left"] + s["width"]))
        else:
            parts.append(("text", t, s["left"], s["left"] + s["width"]))
    out, run = [], None
    def close():
        nonlocal run
        if run is not None:
            out.append("$" + run + "$"); run = None
    for i, (kind, t, l, r) in enumerate(parts):
        prev_r = parts[i - 1][3] if i else -99
        nxt = parts[i + 1] if i + 1 < len(parts) else None
        if kind in ("math", "script"):
            run = t if run is None else run + t
            continue
        core = t.strip()
        gap = l - prev_r
        mathy_next = nxt is not None and nxt[0] in ("math", "script") and nxt[2] - r <= 10
        glue = set("()[]{}|=+-*/<>0123456789")
        if run is not None and core and (
                (gap <= 3 and all(c in glue or c in SYM for c in core)) or # touching, like G(S), v_1, G-e.
                (gap <= 10 and all(c in "=<>+-*/" or c in SYM for c in core)) or # like n >= 3 and k = 3.
                (gap <= 10 and core.isdigit() and prev_ok(out, run))):
            run += "".join(SYM.get(c, c) for c in core)
            continue
        if run is None and core and mathy_next and all(c in "([{" for c in core) and (nxt[2] - r) <= 3:
            run = core; continue # ( right before a variable.
        close()
        out.append(t)
    close()
    s = "".join(out)
    s = re.sub(r"\$\s*\$", "", s)
    return re.sub(r"\s+", " ", s).strip()


def main(n_pages=40, seed=7):
    # pages with exercise numbers, from the text layer.
    pages = subprocess.run(["pdftotext", "-layout", BOOK, "-"], capture_output=True, text=True, errors="ignore").stdout.split("\f")
    cand = [i + 1 for i, p in enumerate(pages) if re.search(r"^\s*\d{1,2}\.\d{1,2}\s+(Prove|Show|Let|Determine|Find|Give|Verify)", p, re.M)]
    random.Random(seed).shuffle(cand)
    cases, used_pages = [], set()
    for pno in cand:
        if len(cases) >= n_pages:
            break
        w, h, spans, images = page_xml(pno)
        lines = lines_of(spans)
        for k, ln in enumerate(lines):
            ltxt = "".join(sp["text"] for sp in ln["spans"]).strip()
            if not re.match(r"^\d{1,2}\.\d{1,2} [A-Z]", ltxt):
                continue
            block = [ln]
            for nxt in lines[k + 1:]:
                text = "".join(s["text"] for s in nxt["spans"]).strip()
                if nxt["top"] - block[-1]["bottom"] > 14 or re.match(r"^\d{1,2}\.\d{1,2}\s", text) or text.startswith("(") or nxt["left"] > block[0]["left"] + 80:
                    break
                block.append(nxt)
            if not (1 <= len(block) <= 6) or pno in used_pages:
                continue
            x0, x1 = min(l["left"] for l in block), max(l["right"] for l in block)
            y0, y1 = min(l["top"] for l in block), max(l["bottom"] for l in block)
            if any(not (ix + iw < x0 or ix > x1 or it + ih < y0 or it > y1) for it, ix, iw, ih in images):
                continue # has an inline image so its math isn't in the text layer.
            truth = ""
            for l in block:
                t = line_truth(l)
                truth = truth + t if truth.endswith("-") and not truth.endswith(" -") else (truth + " " + t if truth else t)
            if any(re.search(r"[A-Za-z]{3,}", m) for m in re.findall(r"\$(.*?)\$", truth)):
                continue # an italic word is emphasis, plain text.
            if not truth.rstrip().endswith((".", "?")) or "$" not in truth or not truth.isascii() or "\\" in truth.replace("\\le", "").replace("\\ge", ""):
                continue # ASCII plain math cases only for this first pass.
            cases.append(dict(page=pno, lines=len(block), truth=truth, box=(x0, y0, x1, y1), scale=(w, h)))
            used_pages.add(pno)
            break
    # render and crop.
    for i, c in enumerate(cases, 1):
        pno = c["page"]
        png = os.path.join(OUT, f"_p{pno}.png")
        if not os.path.exists(png):
            subprocess.run(["pdftoppm", "-f", str(pno), "-l", str(pno), "-r", str(DPI), "-png", "-singlefile", BOOK, png[:-4]], check=True)
        w, h = c["scale"]
        sx = DPI / 72 / 1.5 # pdftohtml coordinates are 1.5x points.
        x0, y0, x1, y1 = [v * sx for v in c["box"]]
        m = 10
        geom = f"{int(x1 - x0 + 2 * m)}x{int(y1 - y0 + 2 * m)}+{int(x0 - m)}+{int(y0 - m)}"
        name = f"c{i:02d}_p{pno}_{c['lines']}l"
        subprocess.run(["magick", png, "-crop", geom, "+repage", os.path.join(OUT, name + ".png")], check=True)
        open(os.path.join(OUT, name + ".txt"), "w").write(c["truth"] + "\n")
        c["name"] = name
    json.dump(cases, open(os.path.join(OUT, "cases.json"), "w"), indent=1)
    for c in cases:
        print(f"{c['name']}: {c['truth']}")


if __name__ == "__main__":
    import argparse
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("pdf"); ap.add_argument("--out", default="tools/fixtures/private"); ap.add_argument("--n", type=int, default=40)
    a = ap.parse_args()
    BOOK, OUT = os.path.abspath(a.pdf), os.path.abspath(a.out)
    os.makedirs(OUT, exist_ok=True)
    main(a.n)
