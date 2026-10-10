//! Real-world LaTeX: equations, lists and tables cut out of CC-BY arXiv papers.
//!
//! `fetch_arxiv` downloads the sources of recent papers, but only ones whose arXiv page shows a CC-BY,
//! CC-BY-SA or CC0 license, pausing between requests as arXiv asks. `corpus_cases` then cuts each
//! equation, align, itemize, enumerate and tabular environment out of the .tex files, gives it the
//! paper's own macro definitions so `\R` and friends work, compiles it, and keeps what renders.
//!
//! The downloaded sources and the pictures stay in `tools/fixtures/` (git-ignored).

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use anyhow::{Context, Result, bail};

use crate::{
    manifest::{Case, Grading},
    render::{degrade, render_latex},
    rng::Rng,
    util::{fnv, par_map, tools_dir},
};

const UA: &str =
    "texcopy-benchmark/0.1 (small local test corpus; https://github.com/Phoenixxo/texcopy)";
const DELAY: Duration = Duration::from_secs(3);

const ENVIRONMENTS: [&str; 10] = [
    "equation",
    "equation*",
    "align",
    "align*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "itemize",
    "enumerate",
];

/// A picture of these would not match its source: unresolved references, figures, files.
const UNUSABLE: [&str; 21] = [
    r"\ref",
    r"\eqref",
    r"\cref",
    r"\Cref",
    r"\autoref",
    r"\cite",
    r"\footnote",
    r"\includegraphics",
    r"\input",
    r"\include",
    r"\url",
    r"\href",
    r"\tikz",
    r"\bibitem",
    r"\caption",
    r"\vspace",
    r"\hspace",
    r"\color",
    r"\textcolor",
    r"\begin{figure",
    r"\begin{tikz",
];

pub fn corpus_dir() -> PathBuf {
    tools_dir().join("fixtures").join("corpus")
}

fn category_of(env: &str) -> &'static str {
    match env {
        "itemize" | "enumerate" => "corpus-list",
        "tabular" => "corpus-table",
        _ => "corpus-equation",
    }
}

// ---- downloading ----------------------------------------------------------------------------

/// Download with curl: it uses the system's certificates and exists on every platform we support.
fn get(url: &str) -> Result<Vec<u8>> {
    let out = Command::new("curl")
        .args(["-sSLf", "-A", UA, "--max-time", "120", url])
        .output()
        .context("running curl")?;
    if !out.status.success() {
        bail!(
            "download failed: {url}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

fn get_text(url: &str) -> Result<String> {
    Ok(String::from_utf8_lossy(&get(url)?).into_owned())
}

/// The text between `<name>` and `</name>`.
fn tag<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let start = s.find(&format!("<{name}>"))? + name.len() + 2;
    let len = s[start..].find(&format!("</{name}>"))?;
    Some(s[start..start + len].trim())
}

/// `http://arxiv.org/abs/2610.12326v1` -> `2610.12326`.
fn arxiv_id(url: &str) -> String {
    let id = url.rsplit("/abs/").next().unwrap_or(url);
    match id.rfind('v') {
        Some(i) if i > 0 && id[i + 1..].chars().all(|c| c.is_ascii_digit()) && i + 1 < id.len() => {
            id[..i].to_string()
        }
        _ => id.to_string(),
    }
}

/// The Creative Commons license link on an arXiv abstract page, if it is one we can reuse.
fn open_license(page: &str) -> Result<String, Option<String>> {
    let Some(i) = page.find("creativecommons.org/") else {
        return Err(None);
    };
    let start = page[..i].rfind('"').map_or(0, |q| q + 1);
    let end = start + page[start..].find('"').unwrap_or(0);
    let url = page[start..end].to_string();
    if url.contains("/licenses/by/")
        || url.contains("/licenses/by-sa/")
        || url.contains("/publicdomain/zero/")
    {
        Ok(url)
    } else {
        Err(Some(url))
    }
}

/// Download papers (up to `count` in the corpus) whose license allows reuse. Returns how many are there.
pub fn fetch_arxiv(count: usize, categories: &[String]) -> Result<usize> {
    let dir = corpus_dir();
    fs::create_dir_all(&dir)?;
    let mut kept = fs::read_dir(&dir)?
        .flatten()
        .filter(|e| e.path().is_dir())
        .count();
    let (mut scanned, mut turn) = (0usize, 0usize);
    let mut starts = vec![0usize; categories.len()];
    while kept < count && scanned < count * 25 {
        let c = turn % categories.len();
        turn += 1;
        let feed = get_text(&format!(
            "http://export.arxiv.org/api/query?search_query=cat:{}&start={}&max_results=10&sortBy=submittedDate&sortOrder=descending",
            categories[c], starts[c]
        ))?;
        starts[c] += 10;
        std::thread::sleep(DELAY);
        for entry in feed.split("<entry>").skip(1) {
            if kept >= count {
                break;
            }
            let Some(id_url) = tag(entry, "id") else {
                continue;
            };
            let id = arxiv_id(id_url);
            scanned += 1;
            let folder = dir.join(id.replace('/', "_"));
            if folder.exists() {
                continue;
            }
            let title = tag(entry, "title")
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let page = get_text(&format!("https://arxiv.org/abs/{id}"))?;
            std::thread::sleep(DELAY);
            let license = match open_license(&page) {
                Ok(url) => url,
                Err(other) => {
                    eprintln!(
                        "  skip {id}: {}",
                        other.unwrap_or_else(|| "no open license".into())
                    );
                    continue;
                }
            };
            let data = get(&format!("https://arxiv.org/e-print/{id}"))?;
            std::thread::sleep(DELAY);
            if save_tex_files(&data, &folder)? == 0 {
                fs::remove_dir_all(&folder).ok();
                eprintln!("  skip {id}: no .tex in the source");
                continue;
            }
            fs::write(
                folder.join("meta.json"),
                serde_json::json!({"id": id, "title": title, "license": license}).to_string(),
            )?;
            kept += 1;
            eprintln!(
                "  kept {id} ({license}): {}",
                title.chars().take(60).collect::<String>()
            );
        }
    }
    Ok(kept)
}

/// Write the .tex files of an e-print (a tarball, or one gzipped file) into `folder`. Returns how many.
fn save_tex_files(data: &[u8], folder: &Path) -> Result<usize> {
    fs::create_dir_all(folder)?;
    let bytes = if data.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(data)
            .take(200_000_000)
            .read_to_end(&mut out)
            .ok();
        out
    } else {
        data.to_vec()
    };
    let mut count = 0;
    let mut archive = tar::Archive::new(Cursor::new(&bytes));
    if let Ok(entries) = archive.entries() {
        for entry in entries.flatten().take(2000) {
            let mut entry = entry;
            let path = entry
                .path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if entry.header().entry_type().is_file()
                && path.ends_with(".tex")
                && entry.size() < 4_000_000
                && !path.starts_with('/')
                && !path.contains("..")
            {
                let mut content = Vec::new();
                entry.read_to_end(&mut content)?;
                let name: String = path
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || "_.-".contains(c) {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                fs::write(folder.join(name), content)?;
                count += 1;
            }
        }
    }
    if count == 0 && String::from_utf8_lossy(&bytes).contains("\\begin{document}") {
        fs::write(folder.join("main.tex"), &bytes)?;
        count = 1;
    }
    Ok(count)
}

// ---- cutting environments out -----------------------------------------------------------------

fn strip_comments(text: &str) -> String {
    text.lines()
        .map(|line| {
            let mut prev = ' ';
            for (i, c) in line.char_indices() {
                if c == '%' && prev != '\\' {
                    return &line[..i];
                }
                prev = c;
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn at(s: &str, i: usize) -> &str {
    s.get(i..).unwrap_or("")
}

/// The index just after the `{...}` group that opens at `s[start]`.
fn balanced(s: &str, start: usize) -> Option<usize> {
    let b = s.as_bytes();
    if b.get(start) != Some(&b'{') {
        return None;
    }
    let (mut depth, mut i) = (0i32, start);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1, // skip the escaped character
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn skip_space(s: &str, mut i: usize) -> usize {
    while s.as_bytes().get(i).is_some_and(|b| b" \t\n".contains(b)) {
        i += 1;
    }
    i
}

/// End of the control word that starts at `s[i]` (`\name`).
fn control_word_end(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    if b.get(i) != Some(&b'\\') {
        return None;
    }
    let mut j = i + 1;
    while b.get(j).is_some_and(u8::is_ascii_alphabetic) {
        j += 1;
    }
    (j > i + 1).then_some(j)
}

/// The paper's own `\newcommand` / `\DeclareMathOperator` / `\def` lines, so its shorthand compiles.
fn macro_definitions(preamble: &str) -> String {
    const KINDS: [&str; 5] = [
        "newcommand",
        "renewcommand",
        "providecommand",
        "DeclareMathOperator",
        "def",
    ];
    let mut out = Vec::new();
    for (idx, _) in preamble.match_indices('\\') {
        let rest = at(preamble, idx + 1);
        let Some(kind) = KINDS.iter().find(|k| {
            rest.starts_with(**k)
                && !at(rest, k.len()).starts_with(|c: char| c.is_ascii_alphabetic())
        }) else {
            continue;
        };
        let mut i = idx + 1 + kind.len();
        if at(preamble, i).starts_with('*') {
            i += 1;
        }
        i = skip_space(preamble, i);
        let end = if *kind == "def" {
            let Some(name_end) = control_word_end(preamble, i) else {
                continue;
            };
            let Some(brace) = at(preamble, name_end).find('{').map(|k| name_end + k) else {
                continue;
            };
            if !preamble[name_end..brace]
                .chars()
                .all(|c| c == '#' || c.is_ascii_digit())
            {
                continue; // a \def pattern we don't handle
            }
            balanced(preamble, brace)
        } else {
            let name_end = if at(preamble, i).starts_with('{') {
                balanced(preamble, i)
            } else {
                control_word_end(preamble, i)
            };
            let Some(mut j) = name_end else { continue };
            j = skip_space(preamble, j);
            while at(preamble, j).starts_with('[') {
                // [number of arguments] and [default]
                let Some(close) = at(preamble, j).find(']') else {
                    break;
                };
                j = skip_space(preamble, j + close + 1);
            }
            if at(preamble, j).starts_with('{') {
                balanced(preamble, j)
            } else {
                None
            }
        };
        let Some(end) = end else { continue };
        let text = preamble[idx..end].trim();
        if text.len() > 10 && text.len() < 400 && !text.contains("\n\n") {
            // an existing name must not stop the build
            out.push(match text.strip_prefix("\\newcommand") {
                Some(rest) => format!("\\providecommand{rest}"),
                None => text.to_string(),
            });
        }
    }
    out.join("\n")
}

/// The environments we care about, whole (with their `\begin` and `\end`).
fn environments(body: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for env in ENVIRONMENTS.iter().copied().chain(["tabular"]) {
        let (open, close) = (format!("\\begin{{{env}}}"), format!("\\end{{{env}}}"));
        for (start, _) in body.match_indices(&open) {
            let (mut depth, mut i) = (0i32, start);
            while i < body.len() {
                let next_open = at(body, i).find(&open).map(|k| i + k);
                let Some(next_close) = at(body, i).find(&close).map(|k| i + k) else {
                    break;
                };
                if next_open.is_some_and(|o| o < next_close) {
                    depth += 1;
                    i = next_open.unwrap() + open.len();
                } else {
                    depth -= 1;
                    i = next_close + close.len();
                    if depth == 0 {
                        found.push((env.to_string(), body[start..i].to_string()));
                        break;
                    }
                }
            }
        }
    }
    found
}

/// Remove `\label{...}`: harmless to drop, and it keeps the text cleaner.
fn without_labels(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("\\label{") {
        out.push_str(&rest[..i]);
        match balanced(rest, i + "\\label".len()) {
            Some(end) => rest = &rest[end..],
            None => {
                rest = &rest[i + 1..];
                out.push('\\');
            }
        }
    }
    out.push_str(rest);
    out
}

struct Candidate {
    paper: String,
    env: String,
    text: String,
    macros: String,
}

fn candidates() -> Vec<Candidate> {
    let mut out = Vec::new();
    let Ok(papers) = fs::read_dir(corpus_dir()) else {
        return out;
    };
    let mut papers: Vec<_> = papers.flatten().filter(|e| e.path().is_dir()).collect();
    papers.sort_by_key(|e| e.file_name());
    for paper in papers {
        let mut files: Vec<_> = fs::read_dir(paper.path())
            .into_iter()
            .flatten()
            .flatten()
            .collect();
        files.sort_by_key(|e| e.file_name());
        for file in files {
            if file.path().extension().is_none_or(|e| e != "tex") {
                continue;
            }
            let text = strip_comments(&String::from_utf8_lossy(
                &fs::read(file.path()).unwrap_or_default(),
            ));
            let Some((preamble, body)) = text.split_once("\\begin{document}") else {
                continue;
            };
            let macros = macro_definitions(preamble);
            for (env, src) in environments(body) {
                let src = without_labels(&src).trim().to_string();
                let limit = if env == "itemize" || env == "enumerate" || env == "tabular" {
                    900
                } else {
                    700
                };
                if UNUSABLE.iter().any(|u| src.contains(u))
                    || !(12 < src.len() && src.len() < limit)
                {
                    continue;
                }
                out.push(Candidate {
                    paper: paper.file_name().to_string_lossy().into_owned(),
                    env,
                    text: src,
                    macros: macros.clone(),
                });
            }
        }
    }
    out
}

/// Render up to `count` corpus cases into `out_dir`.
pub fn corpus_cases(seed: u64, count: usize, out_dir: &Path, jobs: usize) -> Vec<Case> {
    let mut all = candidates();
    let found = all.len();
    let mut rng = Rng::from_text(&format!("corpus-{seed}"));
    rng.shuffle(&mut all);
    // a cap per paper so one paper does not dominate (it grows when few papers have usable text)
    let papers = all
        .iter()
        .map(|c| c.paper.as_str())
        .collect::<HashSet<_>>()
        .len()
        .max(1);
    let cap = 6.max(2 * count / papers);
    let mut per_paper: HashMap<String, usize> = HashMap::new();
    let picked: Vec<(usize, Candidate)> = all
        .into_iter()
        .filter(|c| {
            let n = per_paper.entry(c.paper.clone()).or_insert(0);
            *n += 1;
            *n <= cap
        })
        .take(count * 3)
        .enumerate()
        .collect();

    let results = par_map(picked, jobs, |(i, c)| {
        let category = category_of(&c.env);
        let name = format!("{category}-{i:04}");
        let case_seed = format!("{seed}-corpus-{i}");
        let mut r = Rng::from_text(&case_seed);
        let dpi = *r.pick(&[150u32, 200, 250]);
        let png = out_dir.join(format!("{name}.png"));
        let rendered = render_latex(
            &c.text,
            Grading::Document,
            "latex-cm",
            dpi,
            &png,
            14,
            12,
            &c.macros,
            40,
        );
        if !rendered.is_ok_and(|r| r.is_done()) {
            return None;
        }
        let degraded = if r.chance(0.2) {
            degrade(&png, &mut r).ok().map(str::to_string)
        } else {
            None
        };
        Some((
            fnv(&format!("{}{}", c.text, c.macros)),
            Case {
                name: name.clone(),
                category: category.to_string(),
                kind: Grading::Document,
                truth: c.text,
                renderer: "latex-cm".into(),
                image: format!("{name}.png"),
                dpi,
                width_cm: 14,
                seed: case_seed,
                degraded,
                theme: None,
                font_px: None,
                family: None,
                preamble: Some(c.macros),
                source: Some(c.paper),
            },
        ))
    });
    let mut seen = HashSet::new();
    let cases: Vec<Case> = results
        .into_iter()
        .flatten()
        .filter(|(h, _)| seen.insert(*h))
        .map(|(_, c)| c)
        .take(count)
        .collect();
    let papers_used = cases
        .iter()
        .filter_map(|c| c.source.as_deref())
        .collect::<HashSet<_>>()
        .len();
    eprintln!(
        "  corpus: {} cases from {papers_used} papers ({found} usable environments found)",
        cases.len()
    );
    cases
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macros_of_every_common_kind_are_collected_and_new_becomes_provide() {
        let pre = r"
\newcommand{\R}{\mathbb{R}}
\newcommand\E[1]{\mathbb{E}\left[#1\right]}
\renewcommand{\epsilon}{\varepsilon}
\DeclareMathOperator*{\argmin}{arg\,min}
\def\half{\frac{1}{2}}
\newcommand{\ip}[2]{\langle #1, #2\rangle}
\usepackage{foo}
";
        let m = macro_definitions(pre);
        assert!(m.contains(r"\providecommand{\R}{\mathbb{R}}"), "{m}");
        assert!(
            m.contains(r"\providecommand\E[1]{\mathbb{E}\left[#1\right]}"),
            "{m}"
        );
        assert!(m.contains(r"\renewcommand{\epsilon}{\varepsilon}"), "{m}");
        assert!(
            m.contains(r"\DeclareMathOperator*{\argmin}{arg\,min}"),
            "{m}"
        );
        assert!(m.contains(r"\def\half{\frac{1}{2}}"), "{m}");
        assert!(
            m.contains(r"\providecommand{\ip}[2]{\langle #1, #2\rangle}"),
            "{m}"
        );
        assert!(!m.contains("usepackage"));
    }

    #[test]
    fn environments_are_found_whole_including_nested_ones() {
        let body = r"x \begin{equation} a=\begin{pmatrix}1&2\end{pmatrix} \label{q} \end{equation} \begin{itemize}\item a \begin{itemize}\item b\end{itemize}\end{itemize}";
        let found = environments(body);
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names.iter().filter(|n| **n == "itemize").count(),
            2,
            "outer and inner list: {names:?}"
        );
        assert!(found.iter().any(|(n, t)| n == "equation"
            && t.ends_with(r"\end{equation}")
            && t.contains("pmatrix")));
        assert!(!without_labels(&found[0].1).contains("label"));
    }

    #[test]
    fn comments_and_labels_are_removed_but_escaped_percent_signs_stay() {
        assert_eq!(strip_comments("a % note\n50\\% sure"), "a \n50\\% sure");
        assert_eq!(without_labels(r"x=1 \label{eq:a} + 2"), "x=1  + 2");
    }

    #[test]
    fn arxiv_ids_lose_their_version_and_only_open_licenses_pass() {
        assert_eq!(arxiv_id("http://arxiv.org/abs/2610.12326v2"), "2610.12326");
        assert_eq!(
            arxiv_id("http://arxiv.org/abs/math/0301001v1"),
            "math/0301001"
        );
        let page = |l: &str| format!(r#"<a href="{l}">license</a>"#);
        assert!(open_license(&page("http://creativecommons.org/licenses/by/4.0/")).is_ok());
        assert!(open_license(&page("http://creativecommons.org/licenses/by-sa/4.0/")).is_ok());
        assert!(open_license(&page("http://creativecommons.org/publicdomain/zero/1.0/")).is_ok());
        assert!(open_license(&page("http://creativecommons.org/licenses/by-nc-nd/4.0/")).is_err());
        assert!(open_license("<html>no license link</html>").is_err());
    }
}
