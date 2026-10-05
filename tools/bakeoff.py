#!/usr/bin/env python3
"""Bake-off: score vision-language OCR models on bench.py's cases.

Each model runs in its own llama.cpp server (llama-server, GGUF + mmproj files). Every case
image goes to the model with *its own* recommended prompt and sampling settings, the Markdown
it returns is normalised to LaTeX / `$...$` prose, and the result is graded exactly like texcopy's
output, so numbers are comparable with tools/baseline.json.

  python3 tools/bakeoff.py paddle --models /path/to/models --save paddle.json
  python3 tools/bakeoff.py ovis   --models /path/to/models --suite prose

Expects <models>/<name>/model.gguf and <models>/<name>/mmproj.gguf.
"""
import argparse, base64, json, os, re, statistics, subprocess, sys, tempfile, time, urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import bench  # noqa: E402

OVIS_PROMPT = "Extract all readable content from the image in natural human reading order and output the result as a single Markdown document."
CONFIGS = {
    # Prompts and sampling follow each model's own card / reference client.
    "paddle": dict(prompts={"math": "Formula Recognition:", "prose": "OCR:", "text": "OCR:"}, system=None, sampling={}),
    "ovis": dict(prompts={"math": OVIS_PROMPT, "prose": OVIS_PROMPT, "text": OVIS_PROMPT}, system=None, sampling={}),
    "mineru": dict(
        prompts={"math": "\nFormula Recognition:", "prose": "\nText Recognition:", "text": "\nText Recognition:"},
        system="You are a helpful assistant.",
        sampling={"top_k": 1, "presence_penalty": 1.0, "frequency_penalty": 0.05},
    ),
}


def clean_math(text):
    """A formula image should yield the LaTeX body: drop code fences and display delimiters."""
    t = re.sub(r"^```(?:latex|tex)?\s*|\s*```$", "", text.strip())
    m = re.search(r"\$\$(.*?)\$\$", t, re.S) or re.search(r"\\\[(.*?)\\\]", t, re.S) or re.search(r"\\\((.*?)\\\)", t, re.S)
    t = m.group(1) if m else re.sub(r"^\$|\$$", "", t)
    return t.strip()


def clean_prose(text):
    """A sentence should come back as one line of text with `$...$` math."""
    t = re.sub(r"^```\w*\s*|\s*```$", "", text.strip())
    t = re.sub(r"\\\((.*?)\\\)", r"$\1$", t, flags=re.S)
    t = re.sub(r"\$\$(.*?)\$\$", r"$\1$", t, flags=re.S)
    t = re.sub(r"\\\[(.*?)\\\]", r"$\1$", t, flags=re.S)
    return re.sub(r"\s+", " ", t).strip()


TRAIL = ",.;:!?)\"'"
LEAD = "(\"'"


def _looks_math(core):
    if len(core) == 1:
        return core.isalpha() and core.isascii() and core not in "aAI"
    if any(c in core for c in "=<>+^_[]\\"):
        return True
    return any(c.isdigit() for c in core) and any(c.isalpha() for c in core) and not re.search(r"[A-Za-z]{3}", core)


def mark_math(text):
    """The same word rules as src/prose.rs: wrap lone letters, intervals and expressions in $...$.
    Text already inside $...$ is left alone."""
    parts = text.split("$")
    for i in range(0, len(parts), 2):  # even parts are outside any $...$
        def fix(m):
            w = m.group(0)
            core = w.lstrip(LEAD).rstrip(TRAIL)
            if not core or not _looks_math(core):
                return w
            lead = w[: len(w) - len(w.lstrip(LEAD))]
            trail = w[len(lead) + len(core):]
            return f"{lead}${core}${trail}"
        parts[i] = re.sub(r"\S+", fix, parts[i])
    return "$".join(parts)


class Server:
    def __init__(self, name, models_dir, port):
        d = os.path.join(models_dir, name)
        self.cfg, self.port, self.log = CONFIGS[name], port, tempfile.NamedTemporaryFile("w+", suffix=f"-{name}.log", delete=False)
        self.cmd = ["llama-server", "-m", os.path.join(d, "model.gguf"), "--mmproj", os.path.join(d, "mmproj.gguf"),
                    "--port", str(port), "-ngl", "99", "-c", "16384", "-np", "1",
                    "--cache-ram", "0"]  # no prompt cache: it grew memory from 1.4 GB to 3.3 GB over 45 images
        self.proc = None

    def start(self, timeout=240):
        self.proc = subprocess.Popen(self.cmd, stdout=self.log, stderr=self.log)
        end = time.time() + timeout
        while time.time() < end:
            if self.proc.poll() is not None:
                sys.exit(f"llama-server exited early; log: {self.log.name}")
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/health", timeout=2) as r:
                    if r.status == 200:
                        return
            except Exception:
                time.sleep(1)
        sys.exit(f"llama-server not ready after {timeout}s; log: {self.log.name}")

    def rss_mb(self):
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(self.proc.pid)], capture_output=True, text=True).stdout.strip()
        return int(out) / 1024 if out else 0

    def stop(self):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(10)
            except subprocess.TimeoutExpired:
                self.proc.kill()

    def ask(self, image_path, kind, max_tokens):
        data = base64.b64encode(open(image_path, "rb").read()).decode()
        ext = os.path.splitext(image_path)[1].lstrip(".").lower().replace("jpg", "jpeg")
        msgs = []
        if self.cfg["system"]:
            msgs.append({"role": "system", "content": self.cfg["system"]})
        msgs.append({"role": "user", "content": [
            {"type": "image_url", "image_url": {"url": f"data:image/{ext};base64,{data}"}},
            {"type": "text", "text": self.cfg["prompts"][kind]},
        ]})
        body = {"messages": msgs, "temperature": 0.0, "max_tokens": max_tokens, **self.cfg["sampling"]}
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}/v1/chat/completions", json.dumps(body).encode(),
                                     {"Content-Type": "application/json"})
        t0 = time.time()
        with urllib.request.urlopen(req, timeout=300) as r:
            resp = json.load(r)
        ms = (time.time() - t0) * 1000
        choice = resp["choices"][0]
        return choice["message"]["content"] or "", choice.get("finish_reason"), ms, resp.get("usage", {})


class VlmBench(bench.Bench):
    """bench.Bench with the recogniser swapped for a llama.cpp server."""

    def __init__(self, server, mark, *a, **k):
        super().__init__(*a, **k)
        self.server, self.mark, self.latencies = server, mark, {}

    def texcopy(self, image, kind="math"):
        try:
            raw, finish, ms, usage = self.server.ask(image, kind, 3000 if kind != "math" else 1500)
        except Exception as e:  # timeouts, server errors: count as a failed read, keep going
            return "", {"stop": f"error: {str(e)[:60]}"}
        self.latencies.setdefault(kind, []).append(ms)
        out = clean_math(raw) if kind == "math" else clean_prose(raw)
        if kind == "prose" and self.mark:
            out = mark_math(out)
        return out, {"stop": "eos" if finish == "stop" else finish, "raw": raw, "ms": ms,
                     "tokens_out": usage.get("completion_tokens")}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("model", choices=list(CONFIGS))
    ap.add_argument("--models", required=True, help="folder holding <name>/model.gguf and <name>/mmproj.gguf")
    ap.add_argument("--suite", default="all", choices=["all", "basic", "degraded", "stress", "prose", "real"])
    ap.add_argument("--port", type=int, default=8099)
    ap.add_argument("--mark-math", action="store_true", help="post-process prose with texcopy's math-marking word rules")
    ap.add_argument("--save", help="write per-case results JSON here")
    ap.add_argument("--baseline", default=os.path.join(bench.ROOT, "tools/baseline.json"), help="texcopy results to compare with")
    a = ap.parse_args()

    server = Server(a.model, a.models, a.port)
    suites = {"basic", "degraded", "stress", "prose"} if a.suite == "all" else {a.suite}
    t0 = time.time()
    server.start()
    print(f"server ready in {time.time() - t0:.0f}s", file=sys.stderr)
    try:
        with tempfile.TemporaryDirectory() as work:
            b = VlmBench(server, a.mark_math, "unused", [], work)
            rows = bench.collect(b, suites)
        rss = server.rss_mb()
    finally:
        server.stop()

    bench.summarize(rows)
    print("\nspeed (ms per image, median / max) and memory:")
    for kind, ls in sorted(b.latencies.items()):
        print(f"  {kind:6} n={len(ls):3}  median {statistics.median(ls):6.0f}  max {max(ls):6.0f}")
    print(f"  server memory after the run: {rss:.0f} MB")
    if os.path.exists(a.baseline):
        base = {(r["suite"], r["case"]): r["grade"] for r in json.load(open(a.baseline))}
        common = [r for r in rows if (r["suite"], r["case"]) in base and r["grade"] != "info"]
        better = sum(bench.RANK[r["grade"]] < bench.RANK[base[(r["suite"], r["case"])]] for r in common)
        worse = sum(bench.RANK[r["grade"]] > bench.RANK[base[(r["suite"], r["case"])]] for r in common)
        print(f"\nvs texcopy on the same {len(common)} cases: {better} better, {worse} worse")
    if a.save:
        json.dump(rows, open(a.save, "w"), indent=1)
        print(f"saved {a.save}")


if __name__ == "__main__":
    main()
