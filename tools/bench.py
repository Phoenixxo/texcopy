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
FONT_PRE = {"palatino": r"\usepackage{mathpazo}", "times": r"\usepackage{newtxtext,newtxmath}"}

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

    def render(self, body, pre="", dpi=200, raw=False):
        self.n += 1
        name = f"r{self.n}"
        wrapped = body if raw else r"\[ " + body + r" \]"
        tex = (r"\documentclass{article}\usepackage{amsmath,amssymb,array}" + pre +
               r"\pagestyle{empty}\begin{document}\noindent " + wrapped + "\n" + r"\end{document}" + "\n")
        open(os.path.join(self.work, name + ".tex"), "w").write(tex)
        r = subprocess.run(["pdflatex", "-interaction=batchmode", "-halt-on-error", name + ".tex"],
                           cwd=self.work, capture_output=True, timeout=120)
        if r.returncode:
            return None
        png = os.path.join(self.work, name + ".png")
        run("pdftoppm", "-r", str(dpi), "-png", "-singlefile", os.path.join(self.work, name + ".pdf"), png[:-4])
        run("magick", png, "-trim", "+repage", "-bordercolor", "white", "-border", "12", png)
        return png

    def degrade(self, src_png, args, ext="png"):
        self.n += 1
        out = os.path.join(self.work, f"d{self.n}.{ext}")
        run("magick", src_png, *args, out)
        return out

    def texcopy(self, image):
        r = run(self.binary, image, "--no-copy", *self.tool_args, env=self.env)
        return r.stdout.strip()

    def grade(self, ref_png, true_src, output):
        norm = lambda s: re.sub(r"\\left|\\right|\\,|\\displaystyle|\s+|[{}]", "", s)
        sim = difflib.SequenceMatcher(None, norm(true_src), norm(output)).ratio()
        out_png = self.render(output) if output else None
        if not out_png:
            return "FAIL", sim
        cmp = run("magick", "compare", "-metric", "AE", ref_png, out_png, "null:")
        if cmp.stderr.strip().startswith("0"):
            return "EXACT", sim
        return ("close" if sim >= 0.9 else "differs"), sim


def collect(b, suites):
    rows = []

    def add(suite, case, ref, src, image):
        out = b.texcopy(image)
        g, sim = b.grade(ref, src, out)
        rows.append({"suite": suite, "case": case, "grade": g, "sim": round(sim, 3), "output": out})
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
            for nm, pre in FONT_PRE.items():
                img = b.render(s, pre=pre)
                if img:
                    add("degraded", f"{s[:20]} font-{nm}", ref, s, img)
    if "stress" in suites:
        for name, (s, pre) in STRESS.items():
            ref = b.render(s, pre=pre)
            add("stress", name, ref, s, ref)
        for name, s in INFO_ONLY.items():
            img = b.render(s, raw=True)
            out = b.texcopy(img)
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
    ap.add_argument("--suite", default="all", choices=["all", "basic", "degraded", "stress"])
    ap.add_argument("--tool-args", default="", help="extra flags for texcopy, e.g. '--fit pad'")
    ap.add_argument("--save", help="write results JSON here")
    ap.add_argument("--baseline", help="compare against a saved results JSON")
    ap.add_argument("--verbose", action="store_true", help="print every non-EXACT output")
    a = ap.parse_args()

    missing = [t for t in ("pdflatex", "pdftoppm", "magick") if not shutil.which(t)]
    if missing or not os.path.exists(a.bin):
        sys.exit(f"missing: {', '.join(missing) or a.bin} (build with `cargo build --release`)")
    suites = {"basic", "degraded", "stress"} if a.suite == "all" else {a.suite}
    with tempfile.TemporaryDirectory() as work:
        rows = collect(Bench(a.bin, shlex.split(a.tool_args), work), suites)
    summarize(rows)
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
