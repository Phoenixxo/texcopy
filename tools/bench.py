#!/usr/bin/env python3
"""Regression benchmark for texcopy.

For each case: render known LaTeX to an image with pdflatex, run texcopy on the
image, compile texcopy's output, and compare the two renders pixel by pixel.

  EXACT    re-render is pixel-identical to the original
  close    compiles, differs, but the text is >= 90% similar
  differs  compiles, but is a different formula
  FAIL     does not compile

Needs: pdflatex, pdftoppm (poppler) and ImageMagick (`magick`) on PATH.

  python3 tools/bench.py                          # all suites
  python3 tools/bench.py --suite stress
  python3 tools/bench.py --save new.json          # record results
  python3 tools/bench.py --baseline tools/baseline.json   # report changes
  python3 tools/bench.py --tool-args "--fit pad"  # pass flags to texcopy
"""
import argparse, collections, difflib, json, os, re, shlex, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RANK = {"EXACT": 0, "close": 1, "differs": 2, "FAIL": 3}
WIDE_PAGE = r"\usepackage[paperwidth=120cm,paperheight=8cm,margin=1cm]{geometry}"

BASIC = {
    "T1 simple": [
        r"E = mc^2",
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        r"\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}",
        r"\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}",
    ],
    "T2 medium": [
        r"e^{i\pi} + 1 = 0",
        r"f(z_0) = \frac{1}{2\pi i} \oint_{\gamma} \frac{f(z)}{z - z_0} \, dz",
        r"i\hbar \frac{\partial}{\partial t} \Psi(\mathbf{r},t) = \left[ -\frac{\hbar^2}{2m}\nabla^2 + V(\mathbf{r},t) \right] \Psi(\mathbf{r},t)",
        r"\binom{n}{k} = \frac{n!}{k!(n-k)!}",
        r"\det \begin{vmatrix} a & b & c \\ d & e & f \\ g & h & i \end{vmatrix}",
        r"\Gamma(z) = \int_0^\infty t^{z-1} e^{-t} \, dt",
        r"P(A \mid B) = \frac{P(B \mid A) P(A)}{P(B)}",
    ],
    "T3 long/nested": [
        r"e^x = 1 + x + \frac{x^2}{2!} + \frac{x^3}{3!} + \frac{x^4}{4!} + \cdots + \frac{x^n}{n!} + \cdots",
        r"\sum_{k=0}^{n} \binom{n}{k} x^{k} y^{n-k} = (x+y)^n = \sum_{j=0}^{n} \frac{n!}{j!(n-j)!} x^{j} y^{n-j} + R_n(x,y)",
        r"x = a_0 + \cfrac{1}{a_1 + \cfrac{1}{a_2 + \cfrac{1}{a_3 + \cdots}}}",
        r"\sqrt{1 + \sqrt{1 + \sqrt{1 + \frac{1}{1 + \frac{1}{x}}}}}",
    ],
    "T4 multi-line": [
        r"\begin{aligned} (a+b)^2 &= a^2 + 2ab + b^2 \\ (a-b)^2 &= a^2 - 2ab + b^2 \end{aligned}",
        r"\begin{cases} x + y = 2 \\ x - y = 0 \end{cases}",
        r"\begin{bmatrix} 1 & 2 & \cdots & n \\ 2 & 4 & \cdots & 2n \\ \vdots & \vdots & \ddots & \vdots \\ n & 2n & \cdots & n^2 \end{bmatrix}",
    ],
}

# Each base formula is degraded in the ways a real screenshot might be.
DEGRADE_BASES = [
    r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
    r"f(z_0) = \frac{1}{2\pi i} \oint_{\gamma} \frac{f(z)}{z - z_0} \, dz",
]
DEGRADE_OPS = {  # name -> ImageMagick args
    "down50%": ["-resize", "50%"],
    "down25%": ["-resize", "25%"],
    "blur": ["-blur", "0x1.5"],
    "dark": ["-negate"],
    "margin80": ["-bordercolor", "white", "-border", "80"],
    "yellow-bg": ["-fuzz", "10%", "-fill", "#fff3b0", "-opaque", "white"],
    "noise": ["+noise", "Gaussian", "-attenuate", "0.3"],
}
# Loaded before amsmath (their documented order). Only fonts whose metrics are installed here: most
# others (Palatino, Utopia, Charter, ...) render a blank page on this machine, which is detected and skipped.
FONT_EARLY = {"times": r"\usepackage{mathptmx}", "txfonts": r"\usepackage{txfonts}", "lmodern": r"\usepackage{lmodern}"}

STRESS = {
    **{f"wide {k} terms": (" + ".join(f"a_{{{i}}} x_{{{i}}}" for i in range(1, k + 1)), WIDE_PAGE) for k in (3, 6, 10, 14, 20, 28)},
    **{f"aligned {k} rows": (r"\begin{aligned} " + r" \\ ".join(f"x_{{{i}}} &= a_{{{i}}} + b_{{{i}}}^2" for i in range(1, k + 1)) + r" \end{aligned}", "") for k in (3, 4, 6)},
    "matrix 3x3": (r"\begin{pmatrix} 1 & 2 & 3 \\ 4 & 5 & 6 \\ 7 & 8 & 9 \end{pmatrix}", ""),
    "matrix 5x5": (r"\begin{pmatrix} " + r" \\ ".join(" & ".join(str((r * 5 + c) % 9 + 1) for c in range(5)) for r in range(5)) + r" \end{pmatrix}", ""),
    "text in math": (r"\text{for all } x \in \mathbb{R}, \quad f(x) \geq 0 \text{ whenever } x > 0", ""),
    "rare symbols": (r"\mathfrak{sl}_2 \otimes \mathbb{C}, \ \aleph_0 < 2^{\aleph_0}, \ \wp(z), \ \ell^2, \ \nexists \, x \in \varnothing", ""),
    "array diagram": (r"\begin{array}{ccc} A & \to & B \\ \downarrow & & \downarrow \\ C & \to & D \end{array}", ""),
    "triple integral": (r"\iiint_{V} \left( \frac{\partial^2 u}{\partial x^2} + \frac{\partial^2 u}{\partial y^2} + \frac{\partial^2 u}{\partial z^2} \right) dV = \oint_{\partial V} \nabla u \cdot \mathbf{n} \, dS", ""),
}
# Sentences with inline math, as in a textbook problem set. Rendered in a Times-like font at two
# line widths (one long line vs a two-line block) and two resolutions.
PROSE = [
    r"5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$.",
    r"Let $G$ be a connected graph with $n$ vertices. Show that $G$ has at least $n-1$ edges.",
    r"Prove that for every integer $n \geq 1$, the sum $1 + 2 + \cdots + n$ equals $n(n+1)/2$.",
    r"Find all real numbers $x$ such that $x^2 - 5x + 6 = 0$.",
    r"If $f$ is continuous on $[a,b]$, then $f$ attains a maximum value on $[a,b]$.",
]
PROSE_EARLY = r"\usepackage{mathptmx}"  # must load before amsmath/amssymb
# A real screenshot from a textbook (kept as a fixture) and the text it contains.
PROSE_FIXTURES = {
    "fixtures/textbook-5.4.png": PROSE[0],
}

# Whole-document elements: equations, lists, headings, theorems, tables, special characters...
# The goal is NOT to recover the author's source (the picture can't tell \textbf from {\bf }), only
# to produce LaTeX that renders the same. So these are graded by rendering the output in the same
# width and comparing pixels, with a generous common preamble.
DOC_PRE = (r"\usepackage{amsthm,booktabs}\newtheorem{theorem}{Theorem}\newtheorem{lemma}[theorem]{Lemma}"
           r"\theoremstyle{definition}\newtheorem{definition}[theorem]{Definition}")
DOC_WIDTH_CM = 14
DOCS = {
    "equation numbered": "Equations can be numbered:\n\\begin{equation}\na^2+b^2=c^2\n\\end{equation}\nas we know.",
    "equation numbered, no colon": "The Pythagorean theorem states\n\\begin{equation}\na^2+b^2=c^2\n\\end{equation}\nfor right triangles.",
    "equation*": "We have\n\\begin{equation*}\ne^{i\\pi}+1=0\n\\end{equation*}\nwhich is Euler's identity.",
    "display after colon": "Recall that:\n\\[\n\\sum_{k=1}^{n} k = \\frac{n(n+1)}{2}\n\\]\nThis is Gauss's formula.",
    "display no colon": "The sum\n\\[\n\\sum_{k=1}^{n} k = \\frac{n(n+1)}{2}\n\\]\nequals the triangular number.",
    "align numbered": "We compute\n\\begin{align}\n(a+b)^2 &= a^2 + 2ab + b^2 \\\\\n(a-b)^2 &= a^2 - 2ab + b^2\n\\end{align}\nas required.",
    "align*": "We compute\n\\begin{align*}\n(a+b)^2 &= a^2 + 2ab + b^2 \\\\\n(a-b)^2 &= a^2 - 2ab + b^2\n\\end{align*}\nas required.",
    "cases": "\\[\nf(x)=\\begin{cases} x & x\\ge 0\\\\ -x & x<0\\end{cases}\n\\]",
    "matrix display": "\\[\nA=\\begin{pmatrix}1&2\\\\3&4\\end{pmatrix}\n\\]",
    "inline only": "Let $f(x)=x^2$ and $g(x)=\\sin x$. Then $f\\circ g(x)=\\sin^2 x$.",
    "problem with display": "\\textbf{Problem 3.} Let $G=(V,E)$ be a graph. Show that\n\\[\n\\sum_{v\\in V}\\deg(v)=2|E|.\n\\]",
    "bold italic emph": "This is \\textbf{bold}, this is \\textit{italic}, and this is \\emph{emphasised} text.",
    "small caps underline": "\\textsc{Small Caps} and \\underline{underlined} text.",
    "typewriter": "Use \\texttt{printf} to print a line.",
    "quotes dashes": "He said, ``Hello---world'' and read pages 10--20.",
    "special characters": "Cost is \\$5 \\& 50\\% off, item \\#3, file my\\_name.",
    "accents": "caf\\'e na\\\"ive fa\\c{c}ade",
    "two paragraphs": "The first paragraph is right here.\n\nThe second paragraph follows, with $x$ in it.",
    "centered": "\\begin{center}\nCentred text\n\\end{center}",
    "sections": "\\section*{Introduction}\nThis document is short.\n\\subsection*{Background}\nSome background follows.",
    "itemize": "\\begin{itemize}\n\\item First point\n\\item Second point with $x^2$\n\\item Third point\n\\end{itemize}",
    "enumerate": "\\begin{enumerate}\n\\item Prove the claim.\n\\item Show that $n\\geq 2$.\n\\item Conclude.\n\\end{enumerate}",
    "nested list": "\\begin{itemize}\n\\item Outer one\n\\begin{itemize}\n\\item Inner a\n\\item Inner b\n\\end{itemize}\n\\item Outer two\n\\end{itemize}",
    "description": "\\begin{description}\n\\item[Alpha] The first letter.\n\\item[Beta] The second letter.\n\\end{description}",
    "theorem": "\\begin{theorem}\nEvery bounded monotone sequence converges.\n\\end{theorem}",
    "definition": "\\begin{definition}\nA graph is \\emph{connected} if every two vertices are joined by a path.\n\\end{definition}",
    "proof": "\\begin{proof}\nSuppose $x>0$. Then $x^2>0$.\n\\end{proof}",
    "theorem and proof": "\\begin{lemma}\nIf $n$ is even then $n^2$ is even.\n\\end{lemma}\n\\begin{proof}\nWrite $n=2k$. Then $n^2=4k^2$.\n\\end{proof}",
    "table hlines": "\\begin{tabular}{lcc}\n\\hline\nName & Age & City \\\\\n\\hline\nAnn & 31 & Oslo \\\\\nBob & 28 & Rome \\\\\n\\hline\n\\end{tabular}",
    "table vertical bars": "\\begin{tabular}{|c|c|c|}\n\\hline\nName & Age & City \\\\ \\hline\nAnn & 31 & Oslo \\\\ \\hline\nBob & 28 & Rome \\\\ \\hline\n\\end{tabular}",
    "table booktabs": "\\begin{tabular}{lrr}\n\\toprule\nItem & Qty & Price \\\\\n\\midrule\nPen & 3 & 1.50 \\\\\nBook & 12 & 20.00 \\\\\n\\bottomrule\n\\end{tabular}",
    "table math cells": "\\begin{tabular}{cc}\n\\hline\n$x$ & $x^2$ \\\\\n\\hline\n1 & 1 \\\\\n2 & 4 \\\\\n3 & 9 \\\\\n\\hline\n\\end{tabular}",
    "table multicolumn": "\\begin{tabular}{lcc}\n\\hline\n& \\multicolumn{2}{c}{Scores} \\\\\nName & Math & Art \\\\\n\\hline\nAnn & 90 & 85 \\\\\n\\hline\n\\end{tabular}",
    "verbatim": "\\begin{verbatim}\nint main() {\n  return 0;\n}\n\\end{verbatim}",
    "footnote": "This sentence has a footnote.\\footnote{Here it is.} The text continues.",
    "bibliography": "\\begin{thebibliography}{9}\n\\bibitem{a} A. Author, \\emph{A Book}, 2020.\n\\bibitem{b} B. Writer, A Paper, 2021.\n\\end{thebibliography}",
}

# Not expected to work: reported for information, never graded.
INFO_ONLY = {
    "table": r"\begin{tabular}{|c|c|c|}\hline Name & Age & City \\ \hline Ann & 31 & Oslo \\ Bob & 28 & Rome \\ \hline\end{tabular}",
    "paragraph": r"The quick brown fox jumps over the lazy dog. Euler showed that $e^{i\pi}+1=0$, which many consider beautiful.",
}


def run(*cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


class Bench:
    def __init__(self, binary, tool_args, work):
        self.binary, self.tool_args, self.work, self.n = binary, tool_args, work, 0
        self.env = dict(os.environ)
        self.env.setdefault("TEXCOPY_MODEL_DIR", os.path.join(ROOT, "models"))

    def render(self, body, pre="", dpi=200, raw=False, opts="", early="", strict=True, wrap=None):
        self.n += 1
        name = f"r{self.n}"
        wrapped = body if raw else r"\[ " + body + r" \]"
        if wrap:
            wrapped = wrap(wrapped)
        tex = (r"\documentclass" + opts + r"{article}" + early + r"\usepackage{amsmath,amssymb,array}" + pre +
               r"\pagestyle{empty}\begin{document}\noindent " + wrapped + "\n" + r"\end{document}" + "\n")
        open(os.path.join(self.work, name + ".tex"), "w").write(tex)
        # strict: any LaTeX error is a failure (used for the model's output). Relaxed: a PDF is
        # enough, which tolerates harmless font warnings in our own reference sources.
        flags = ["-halt-on-error"] if strict else []
        r = subprocess.run(["pdflatex", "-interaction=batchmode", *flags, name + ".tex"],
                           cwd=self.work, capture_output=True, timeout=120)
        if r.returncode if strict else not os.path.exists(os.path.join(self.work, name + ".pdf")):
            return None
        png = os.path.join(self.work, name + ".png")
        run("pdftoppm", "-r", str(dpi), "-png", "-singlefile", os.path.join(self.work, name + ".pdf"), png[:-4])
        run("magick", png, "-trim", "+repage", "-bordercolor", "white", "-border", "12", png)
        # A reference with no ink (e.g. a font whose metrics are missing drops every glyph) would make
        # every model "fail" for no reason, so treat it as a render failure.
        if float(run("magick", png, "-format", "%[fx:mean]", "info:").stdout or 0) > 0.9995:
            return None
        return png

    def degrade(self, src_png, args, ext="png"):
        self.n += 1
        out = os.path.join(self.work, f"d{self.n}.{ext}")
        run("magick", src_png, *args, out)
        return out

    def texcopy(self, image, kind="math"):
        """Returns (latex, meta) where meta carries stop reason and confidence.
        `kind` ("math", "prose" or "text") is ignored here; other backends use it to pick a prompt."""
        r = run(self.binary, image, "--json", *self.tool_args, env=self.env)
        try:
            d = json.loads(r.stdout)
        except ValueError:
            return "", {}
        return d["latex"], d

    def grade_doc(self, ref_png, output):
        """Render `output` like the reference and compare pixels. Returns (grade, 1 - differing/ink)."""
        out_png = self.render(output, pre=DOC_PRE, raw=True, opts="[12pt]", strict=True,
                              wrap=lambda b: r"\begin{minipage}{%dcm}%s\end{minipage}" % (DOC_WIDTH_CM, b)) if output else None
        if not out_png:
            return "FAIL", 0.0
        size = lambda p: tuple(int(v) for v in run("magick", "identify", "-format", "%w %h", p).stdout.split())
        (w1, h1), (w2, h2) = size(ref_png), size(out_png)
        geom = f"{max(w1, w2)}x{max(h1, h2)}"
        padded = []
        for p in (ref_png, out_png):
            q = p[:-4] + "-pad.png"
            run("magick", p, "-background", "white", "-gravity", "northwest", "-extent", geom, q)
            padded.append(q)
        ae = float(run("magick", "compare", "-metric", "AE", *padded, "null:").stderr.split()[0])
        ink = float(run("magick", ref_png, "-colorspace", "Gray", "-format", "%[fx:(1-mean)*w*h]", "info:").stdout or 1)
        ratio = ae / max(ink, 1.0)
        return ("EXACT" if ae == 0 else "close" if ratio < 0.15 else "differs"), round(max(0.0, 1 - ratio), 3)

    def grade(self, ref_png, true_src, output, pre=""):
        norm = lambda s: re.sub(r"\\left|\\right|\\,|\\displaystyle|\s+|[{}]", "", s)
        sim = difflib.SequenceMatcher(None, norm(true_src), norm(output)).ratio()
        out_png = self.render(output, pre=pre) if output else None
        if not out_png:
            return "FAIL", sim
        cmp = run("magick", "compare", "-metric", "AE", ref_png, out_png, "null:")
        if cmp.stderr.strip().startswith("0"):
            return "EXACT", sim
        return ("close" if sim >= 0.9 else "differs"), sim


REAL_DIR = os.path.join(ROOT, "tools", "fixtures", "private")


def grade_real(truth, output):
    """Real textbook exercises: same characters *and* same words. Ignores `$`, braces, spacing and the
    LaTeX spelling of things the truth writes plainly (`\\{` for `{`, `\\text{Cen}` for `Cen`)."""
    def strip(t):
        t = t.replace("\u2212", "-").replace("\u2013", "-").replace("\\geq", "\\ge").replace("\\leq", "\\le")
        t = re.sub(r"\\(text|operatorname|mathrm|textit|mathit)\{([^}]*)\}", r"\2", t)
        t = t.replace("\\{", "{").replace("\\}", "}").replace("\\,", "").replace("\\ ", "")
        return re.sub(r"[\s${}]", "", t)
    words = lambda t: re.findall(r"[A-Za-z]{3,}", re.sub(r"\\(text|operatorname|mathrm)", "", t).replace("\\", " "))
    if not output:
        return "FAIL", 0.0
    a, b = strip(truth), strip(output)
    if a == b and words(truth) == words(output):
        return "EXACT", 1.0
    sim = difflib.SequenceMatcher(None, a, b).ratio()
    return ("close" if sim >= 0.95 else "differs"), sim


def grade_text(true_src, output):
    """Prose is graded as text: spacing between words matters (lost spaces make output unusable),
    but spacing inside $...$ does not."""
    def norm(t):
        parts = t.split("$")
        return "$".join(re.sub(r"\s+", "", p) if i % 2 else re.sub(r"\s+", " ", p).strip() for i, p in enumerate(parts))
    if not output:
        return "FAIL", 0.0
    a, b = norm(true_src), norm(output)
    if a == b:
        return "EXACT", 1.0
    sim = difflib.SequenceMatcher(None, a, b).ratio()
    return ("close" if sim >= 0.9 else "differs"), sim


def collect(b, suites):
    rows = []

    def add(suite, case, ref, src, image, pre=""):
        out, meta = b.texcopy(image)
        g, sim = b.grade(ref, src, out, pre)
        rows.append({"suite": suite, "case": case, "grade": g, "sim": round(sim, 3), "output": out,
                     "stop": meta.get("stop"), "min_prob": meta.get("min_prob"), "mean_logprob": meta.get("mean_logprob")})
        print(f"  {g:8} {suite:16} {case}", file=sys.stderr)

    if "basic" in suites:
        for tier, srcs in BASIC.items():
            for s in srcs:
                ref = b.render(s)
                add(tier, s[:48], ref, s, ref)
    if "degraded" in suites:
        for s in DEGRADE_BASES:
            ref = b.render(s)
            for dpi in (50, 100, 400, 800):
                add("degraded", f"{s[:20]} dpi{dpi}", ref, s, b.render(s, dpi=dpi))
            for nm, args in DEGRADE_OPS.items():
                add("degraded", f"{s[:20]} {nm}", ref, s, b.degrade(ref, args))
            add("degraded", f"{s[:20]} jpeg-q20", ref, s, b.degrade(ref, ["-quality", "20"], "jpg"))
            for nm, early in FONT_EARLY.items():
                img = b.render(s, early=early, strict=False)
                if img:
                    add("degraded", f"{s[:20]} font-{nm}", ref, s, img)
                else:
                    print(f"  SKIPPED  degraded         {s[:20]} font-{nm}: LaTeX package not installed?", file=sys.stderr)
    if "prose" in suites:
        def add_prose(case, true_src, image):
            out, meta = b.texcopy(image, kind="prose")
            g, sim = grade_text(true_src, out)
            rows.append({"suite": "prose", "case": case, "grade": g, "sim": round(sim, 3), "output": out,
                         "stop": meta.get("stop"), "min_prob": meta.get("min_prob"), "mean_logprob": meta.get("mean_logprob")})
            print(f"  {g:8} {'prose':16} {case}", file=sys.stderr)

        for rel, true_src in PROSE_FIXTURES.items():
            add_prose("real screenshot " + os.path.basename(rel), true_src, os.path.join(os.path.dirname(os.path.abspath(__file__)), rel))
        for n, sent in enumerate(PROSE, 1):
            for width in (16, 9):
                for dpi in (110, 200):
                    body = r"\begin{minipage}{%dcm}%s\end{minipage}" % (width, sent)
                    img = b.render(body, early=PROSE_EARLY, dpi=dpi, raw=True, opts="[12pt]", strict=False)
                    if img:
                        add_prose(f"s{n} {width}cm {dpi}dpi", sent, img)
                    else:
                        print(f"  SKIPPED  prose            s{n} {width}cm {dpi}dpi: render failed", file=sys.stderr)
    if "documents" in suites:
        for name, body in DOCS.items():
            ref = b.render(body, pre=DOC_PRE, raw=True, opts="[12pt]", strict=False,
                           wrap=lambda x: r"\begin{minipage}{%dcm}%s\end{minipage}" % (DOC_WIDTH_CM, x))
            if not ref:
                print(f"  SKIPPED  documents        {name}: reference failed to render", file=sys.stderr)
                continue
            out, meta = b.texcopy(ref, kind="prose")
            # a bare formula (kind "math") has no delimiters by design; show it as the display it is
            g, sim = b.grade_doc(ref, rf"\[ {out} \]" if meta.get("kind") == "math" and out else out)
            rows.append({"suite": "documents", "case": name, "grade": g, "sim": sim, "output": out, "raw": meta.get("raw"),
                         "stop": meta.get("stop"), "min_prob": meta.get("min_prob"), "mean_logprob": meta.get("mean_logprob")})
            print(f"  {g:8} {'documents':16} {name}", file=sys.stderr)
    if "real" in suites:
        index = os.path.join(REAL_DIR, "cases.json")
        if not os.path.exists(index):
            print("  SKIPPED  real: no tools/fixtures/private/cases.json (make one with tools/book_cases.py)", file=sys.stderr)
        else:
            for c in json.load(open(index)):
                img = os.path.join(REAL_DIR, c["name"] + ".png")
                out, meta = b.texcopy(img, kind="prose")
                g, sim = grade_real(c["truth"], out)
                rows.append({"suite": "real", "case": c["name"], "grade": g, "sim": round(sim, 3), "output": out,
                             "stop": meta.get("stop"), "min_prob": meta.get("min_prob"), "mean_logprob": meta.get("mean_logprob")})
                print(f"  {g:8} {'real':16} {c['name']}", file=sys.stderr)
    if "stress" in suites:
        for name, (s, pre) in STRESS.items():
            ref = b.render(s, pre=pre)
            add("stress", name, ref, s, ref, pre)
        for name, s in INFO_ONLY.items():
            img = b.render(s, raw=True)
            out, _ = b.texcopy(img, kind="text")
            rows.append({"suite": "info", "case": name, "grade": "info", "sim": 0, "output": out})
    return rows


def summarize(rows):
    by = collections.defaultdict(collections.Counter)
    for r in rows:
        if r["grade"] != "info":
            by[r["suite"]][r["grade"]] += 1
    print(f"\n{'suite':16} {'EXACT':>6} {'close':>6} {'differs':>8} {'FAIL':>5}  n")
    tot = collections.Counter()
    for suite, c in by.items():
        tot.update(c)
        print(f"{suite:16} {c['EXACT']:6} {c['close']:6} {c['differs']:8} {c['FAIL']:5}  {sum(c.values())}")
    print(f"{'TOTAL':16} {tot['EXACT']:6} {tot['close']:6} {tot['differs']:8} {tot['FAIL']:5}  {sum(tot.values())}")


def confidence_report(rows):
    """Does low model confidence predict a wrong answer? Helps choose a warning threshold."""
    graded = [r for r in rows if r["grade"] != "info" and r.get("min_prob") is not None]
    if not graded:
        return
    exact = [r for r in graded if r["grade"] == "EXACT"]
    close = [r for r in graded if r["grade"] == "close"]
    wrong = [r for r in graded if r["grade"] in ("differs", "FAIL")]
    print(f"\nconfidence (min token probability): {len(exact)} exact, {len(close)} close, {len(wrong)} wrong")
    print(f"{'warn if min_prob <':20} {'exact flagged':>14} {'close flagged':>14} {'wrong flagged':>14}")
    for t in (0.5, 0.7, 0.8, 0.9, 0.95, 0.99):
        f = lambda grp: f"{sum(r['min_prob'] < t for r in grp)}/{len(grp)}"
        print(f"{t:<20} {f(exact):>14} {f(close):>14} {f(wrong):>14}")
    loops = sum(r.get("stop") == "looped" for r in graded)
    cut = sum(r.get("stop") == "max_tokens" for r in graded)
    print(f"stop reasons: {loops} looped, {cut} hit the token limit")


def compare(rows, path):
    old = {(r["suite"], r["case"]): r["grade"] for r in json.load(open(path))}
    worse, better = [], []
    for r in rows:
        before = old.get((r["suite"], r["case"]))
        if before and before != r["grade"] and "info" not in (before, r["grade"]):
            (worse if RANK[r["grade"]] > RANK[before] else better).append(f"  {r['suite']:16} {r['case']}: {before} -> {r['grade']}")
    print(f"\nvs {path}: {len(better)} improved, {len(worse)} regressed")
    for line in better:
        print("improved" + line)
    for line in worse:
        print("REGRESSED" + line)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=os.path.join(ROOT, "target/release/texcopy"))
    ap.add_argument("--suite", default="all", choices=["all", "basic", "degraded", "stress", "prose", "real", "documents"])
    ap.add_argument("--tool-args", default="", help="extra flags for texcopy, e.g. '--fit pad'")
    ap.add_argument("--save", help="write results JSON here")
    ap.add_argument("--baseline", help="compare against a saved results JSON")
    ap.add_argument("--verbose", action="store_true", help="print every non-EXACT output")
    a = ap.parse_args()

    missing = [t for t in ("pdflatex", "pdftoppm", "magick") if not shutil.which(t)]
    if missing or not os.path.exists(a.bin):
        sys.exit(f"missing: {', '.join(missing) or a.bin} (build with `cargo build --release`)")
    suites = {"basic", "degraded", "stress", "prose", "documents"} | ({"real"} if os.path.exists(os.path.join(REAL_DIR, "cases.json")) else set()) if a.suite == "all" else {a.suite}
    with tempfile.TemporaryDirectory() as work:
        rows = collect(Bench(a.bin, shlex.split(a.tool_args), work), suites)
    summarize(rows)
    confidence_report(rows)
    if a.verbose:
        for r in rows:
            if r["grade"] not in ("EXACT",):
                print(f"\n[{r['suite']}] {r['case']} -> {r['grade']} (sim {r['sim']})\n  {r['output']}")
    if a.baseline:
        compare(rows, a.baseline)
    if a.save:
        json.dump(rows, open(a.save, "w"), indent=1)
        print(f"saved {a.save}")


if __name__ == "__main__":
    main()
