#!/usr/bin/env python3
"""Visual gallery: what texcopy sees, what it writes, and how that renders.

For each case it shows the input image, the LaTeX texcopy produced, that LaTeX
re-rendered with pdflatex (so you can compare by eye), per-token confidence and
the verdict. Output is one self-contained HTML file (images embedded).

  python3 tools/gallery.py              # writes gallery/index.html
  python3 tools/gallery.py --open       # ...and opens it (macOS `open`)

Needs the same tools as bench.py: pdflatex, pdftoppm, ImageMagick (`magick`).
"""
import argparse, base64, html, os, shutil, subprocess, sys, tempfile, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402

WIDE = bench.WIDE_PAGE
CASES = [
    dict(title="Quadratic formula", note="The easy case: a clean render.",
         src=r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}"),
    dict(title="Schrödinger equation", note="Nested fractions, brackets, bold vectors, a differential operator.",
         src=r"i\hbar \frac{\partial}{\partial t} \Psi(\mathbf{r},t) = \left[ -\frac{\hbar^2}{2m}\nabla^2 + V(\mathbf{r},t) \right] \Psi(\mathbf{r},t)"),
    dict(title="Determinant", note="The model emits a generic matrix with delimiters; tidy renames it to a proper vmatrix.",
         src=r"\det \begin{vmatrix} a & b & c \\ d & e & f \\ g & h & i \end{vmatrix}"),
    dict(title="Matrix with ellipses", note="Dots in three directions. Compare the raw model output with the tidied result.",
         src=r"\begin{bmatrix} 1 & 2 & \cdots & n \\ 2 & 4 & \cdots & 2n \\ \vdots & \vdots & \ddots & \vdots \\ n & 2n & \cdots & n^2 \end{bmatrix}"),
    dict(title="System of equations", note="Tidy turns the delimited matrix into a real cases environment.",
         src=r"\begin{cases} x + y = 2 \\ x - y = 0 \end{cases}"),
    dict(title="Continued fraction", note="Deeply nested structure.",
         src=r"x = a_0 + \cfrac{1}{a_1 + \cfrac{1}{a_2 + \cfrac{1}{a_3 + \cdots}}}"),
    dict(title="Dark-mode screenshot", note="Colours are inverted automatically before the model sees the image.",
         src=r"f(z_0) = \frac{1}{2\pi i} \oint_{\gamma} \frac{f(z)}{z - z_0} \, dz", degrade=["-negate"]),
    dict(title="Yellow background + blur", note="A noisy, tinted capture, like a slide or a web page.",
         src=r"\Gamma(z) = \int_0^\infty t^{z-1} e^{-t} \, dt",
         degrade=["-blur", "0x1.2", "-fuzz", "10%", "-fill", "#fff3b0", "-opaque", "white"]),
    dict(title="Tiny screenshot (12% size)", note="Too little detail: the model gets it wrong. Watch the confidence strip and the warning.",
         src=r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}", degrade=["-resize", "12%"]),
    dict(title="Long equation (14 terms)", note="About 29 times wider than tall: near the limit. Mostly right, ends sloppy.",
         src=" + ".join(f"a_{{{i}}} x_{{{i}}}" for i in range(1, 15)), pre=WIDE),
    dict(title="Too long (20 terms)", note="About 44:1. The model gets stuck repeating itself, so the CLI refuses to copy anything.",
         src=" + ".join(f"a_{{{i}}} x_{{{i}}}" for i in range(1, 21)), pre=WIDE),
    dict(title="Two aligned lines", note="Known weak spot: line breaks are lost, and the model is confident about it.",
         src=r"\begin{aligned} (a+b)^2 &= a^2 + 2ab + b^2 \\ (a-b)^2 &= a^2 - 2ab + b^2 \end{aligned}"),
    dict(title="Problem statement", prose=True,
         note="A sentence with inline math, wrapped over two lines. texcopy flattens it into one paragraph and marks the math with $...$.",
         src=bench.PROSE[0], width=9),
    dict(title="Another problem statement", prose=True,
         note="Inline math is found by rules: lone letters, intervals and expressions are math; words are not.",
         src=bench.PROSE[4], width=16),
    dict(title="Three-line paragraph: the current limit", prose=True,
         note="The model reads a tall block at once and loses the spaces between words. texcopy warns instead of hiding it.",
         src=r"5.7 Let $G$ be a connected graph with $n$ vertices and let $v$ be a vertex of $G$. Prove that $G - v$ has at most $\deg(v)$ components, and that equality holds if $v$ is a cut-vertex of a tree.", width=10),
    dict(title="Not a formula: a table", note="Out of scope, but it gets surprisingly close.", raw=True,
         src=r"\begin{tabular}{|c|c|c|}\hline Name & Age & City \\ \hline Ann & 31 & Oslo \\ Bob & 28 & Rome \\ \hline\end{tabular}"),
    dict(title="A longer sentence", prose=True, note="Reads the first line and then stops: one tall image with two lines of different widths is not reliable yet.",
         src=r"The quick brown fox jumps over the lazy dog. Euler showed that $e^{i\pi}+1=0$, which many consider beautiful.", width=16),
]

CSS = """
:root{--bg:#fafaf9;--card:#fff;--fg:#1c1917;--muted:#78716c;--line:#e7e5e4;--ok:#15803d;--warn:#b45309;--bad:#b91c1c;--code:#f5f5f4}
@media (prefers-color-scheme:dark){:root{--bg:#1c1917;--card:#292524;--fg:#f5f5f4;--muted:#a8a29e;--line:#44403c;--ok:#4ade80;--warn:#fbbf24;--bad:#f87171;--code:#1c1917}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 -apple-system,system-ui,sans-serif}
main{max-width:980px;margin:0 auto;padding:24px 16px 64px}h1{font-size:24px;margin:0 0 4px}
.lead{color:var(--muted);margin:0 0 20px}.legend{display:flex;gap:14px;flex-wrap:wrap;font-size:13px;color:var(--muted);margin-bottom:24px}
.card{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:16px;margin-bottom:20px}
.head{display:flex;justify-content:space-between;align-items:baseline;gap:12px;flex-wrap:wrap}.head h2{font-size:17px;margin:0}
.note{color:var(--muted);margin:2px 0 12px;font-size:14px}.badge{font-size:12px;font-weight:600;padding:2px 9px;border-radius:99px;border:1px solid currentColor;white-space:nowrap}
.exact{color:var(--ok)}.close{color:var(--warn)}.differs,.fail,.refused{color:var(--bad)}.info{color:var(--muted)}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:12px}@media(max-width:700px){.pair{grid-template-columns:1fr}}
.pane{border:1px solid var(--line);border-radius:8px;overflow:hidden}.pane .cap{font-size:11px;letter-spacing:.06em;text-transform:uppercase;color:var(--muted);padding:5px 10px;border-bottom:1px solid var(--line)}
.pane .img{background:#fff;padding:10px;display:flex;align-items:center;justify-content:center;min-height:90px}.pane img{max-width:100%;height:auto}
.lbl{font-size:11px;letter-spacing:.06em;text-transform:uppercase;color:var(--muted);margin:12px 0 0}
.none{color:#888;font-size:13px;padding:20px;text-align:center}
pre{background:var(--code);border:1px solid var(--line);border-radius:8px;padding:10px 12px;margin:4px 0 6px;overflow-x:auto;font-size:13px;white-space:pre-wrap;word-break:break-all}
.toks{margin:8px 0;font:12px/2 ui-monospace,Menlo,monospace}.toks span{padding:1px 1px;border-radius:3px}
.t-low{background:rgba(220,38,38,.28)}.t-mid{background:rgba(245,158,11,.25)}
.meta{font-size:13px;color:var(--muted);display:flex;gap:16px;flex-wrap:wrap}.warn{color:var(--warn);font-size:13px;margin-top:6px}
.err{color:var(--bad);font-size:14px;font-weight:600;margin-top:10px}details{margin-top:8px;font-size:13px}summary{cursor:pointer;color:var(--muted)}
"""


def png64(path):
    return base64.b64encode(open(path, "rb").read()).decode()


def pane(caption, path):
    body = f'<img src="data:image/png;base64,{png64(path)}" alt="{html.escape(caption)}">' if path else '<div class="none">nothing to render</div>'
    return f'<div class="pane"><div class="cap">{caption}</div><div class="img">{body}</div></div>'


def token_strip(tokens):
    spans = []
    for t in tokens:
        p = t["prob"]
        cls = "t-low" if p < 0.5 else "t-mid" if p < 0.9 else ""
        spans.append(f'<span class="{cls}" title="{p:.0%} sure">{html.escape(t["text"].strip() or "·")}</span>')
    return '<div class="toks">' + "".join(spans) + "</div>"


def run_case(b, case):
    src, pre = case["src"], case.get("pre", "")
    raw = case.get("raw", False)
    prose = case.get("prose", False)
    if prose and case.get("image"):
        ref = image = case["image"]
    elif prose:
        body = r"\begin{minipage}{%dcm}%s\end{minipage}" % (case.get("width", 16), src)
        ref = image = b.render(body, early=bench.PROSE_EARLY, dpi=200, raw=True, opts="[12pt]", strict=False)
    else:
        ref = b.render(src, pre=pre, raw=raw)
        image = b.degrade(ref, case["degrade"]) if case.get("degrade") else ref
    t0 = time.time()
    out, meta = b.texcopy(image)
    ms = (time.time() - t0) * 1000
    refused = meta.get("stop") == "looped"
    if prose:
        out_png = None if refused or not out else b.render(out, early=bench.PROSE_EARLY, dpi=200, raw=True, opts="[12pt]", strict=False)
        grade, _ = bench.grade_text(src, out)
    else:
        out_png = None if refused or not out else b.render(out, pre=pre)
    if prose:
        pass
    elif raw:
        grade = "info"
    elif refused:
        grade = "refused"
    else:
        grade, _ = b.grade(ref, src, out, pre)
    label = {"EXACT": "exact match", "close": "close", "differs": "different text" if prose else "different formula", "FAIL": "doesn't compile",
             "refused": "refused", "info": "out of scope"}[grade]

    toks = meta.get("tokens", [])
    unsure = [t for t in toks if t["prob"] < 0.5]
    parts = [f'<section class="card"><div class="head"><h2>{html.escape(case["title"])}</h2>'
             f'<span class="badge {grade.lower()}">{label}</span></div><p class="note">{html.escape(case["note"])}</p>',
             '<div class="pair">', pane("Input image", image),
             pane("Output, re-rendered by pdflatex", out_png), "</div>"]
    if refused:
        parts.append('<div class="err">texcopy refused: the model got stuck repeating itself. Nothing is copied.</div>')
    else:
        parts.append(f'<div class="lbl">LaTeX copied to your clipboard</div><pre>{html.escape(out) or "(empty)"}</pre>')
    if toks:
        parts.append('<div class="lbl">What the model wrote, token by token (shaded where it was unsure)</div>' + token_strip(toks))
    if unsure and not refused:
        parts.append(f'<div class="warn">warning: unsure about {len(unsure)} token(s). The CLI would print this and still copy the result.</div>')
    mp = meta.get("min_prob")
    parts.append(f'<div class="meta"><span>{ms:.0f} ms</span>'
                 + (f"<span>least-sure token: {mp:.0%}</span>" if mp is not None else "")
                 + f'<span>stop: {meta.get("stop", "?")}</span></div>')
    if meta.get("raw") is not None:
        parts.append(f'<details><summary>What the model emitted before tidy</summary><pre>{html.escape(meta["raw"])}</pre></details>')
    parts.append("</section>")
    return "".join(parts), grade


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=os.path.join(bench.ROOT, "target/release/texcopy"))
    ap.add_argument("--out", default=os.path.join(bench.ROOT, "gallery/index.html"))
    ap.add_argument("--open", action="store_true")
    a = ap.parse_args()
    missing = [t for t in ("pdflatex", "pdftoppm", "magick") if not shutil.which(t)]
    if missing or not os.path.exists(a.bin):
        sys.exit(f"missing: {', '.join(missing) or a.bin} (build with `cargo build --release`)")

    cards, grades = [], []
    with tempfile.TemporaryDirectory() as work:
        b = bench.Bench(a.bin, [], work)
        for i, case in enumerate(CASES, 1):
            print(f"[{i}/{len(CASES)}] {case['title']}", file=sys.stderr)
            card, g = run_case(b, case)
            cards.append(card)
            grades.append(g)
    exact = grades.count("EXACT")
    page = (f'<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">'
            f"<title>texcopy gallery</title><style>{CSS}</style></head><body><main>"
            f"<h1>texcopy gallery</h1><p class=\"lead\">{len(CASES)} cases, {exact} re-render pixel-identically to the original. "
            "Left: what texcopy was given. Right: its LaTeX output compiled by pdflatex, so you can judge by eye.</p>"
            '<div class="legend"><span>Token confidence:</span><span><span class="toks"><span>normal</span></span> 90%+ sure</span>'
            '<span><span class="toks"><span class="t-mid">amber</span></span> under 90%</span>'
            '<span><span class="toks"><span class="t-low">red</span></span> under 50% (the CLI warns)</span></div>'
            + "".join(cards) + "</main></body></html>")
    os.makedirs(os.path.dirname(a.out), exist_ok=True)
    open(a.out, "w").write(page)
    print(f"wrote {a.out} ({os.path.getsize(a.out) / 1e6:.1f} MB)")
    if a.open:
        subprocess.run(["open", a.out])


if __name__ == "__main__":
    main()
