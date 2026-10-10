//! Prose mode: turn the model's reading of a sentence with inline math into LaTeX text.
//!
//! The model is trained on formulas, so given a line of text it writes the words as
//! `\mathrm{W o r d s ~ l i k e ~ t h i s}` (or `\text{..}`), often wraps the visual lines in
//! an `array`, and does not mark which letters were italic math. This module undoes that:
//! it flattens the rows into one paragraph and wraps the math in `$...$`, using rules
//! because the model gives no italics signal. It cannot tell a variable `a` from the
//! article "a", so `a`, `A` and `I` are always left as words.

use crate::tidy::matching_brace;

#[derive(Debug)]
enum Piece {
    Text(String),
    Math(String),
}

struct Word {
    text: String,
    math: bool,
    space_before: bool,
    /// First word of a visual line after the first (used to repair breaks at line ends).
    row_start: bool,
}

/// Convert compacted model output into prose, or `None` if it doesn't look like a sentence.
/// `math` tidies a math fragment (it is called on the text between the `$` signs).
pub fn convert(compact: &str, math: &dyn Fn(&str) -> String) -> Option<String> {
    let rows = rows(compact)?;
    let mut words: Vec<Word> = Vec::new();
    for (n, row) in rows.iter().enumerate() {
        let mut pieces = Vec::new();
        scan(row, false, &mut pieces);
        let first = words.len();
        let mut space = n > 0;
        let push = |words: &mut Vec<Word>, text: String, math: bool, space: bool| {
            words.push(Word {
                text,
                math,
                space_before: space,
                row_start: n > 0 && words.len() == first,
            });
        };
        for piece in pieces {
            match piece {
                Piece::Text(t) => {
                    let mut cur = String::new();
                    for ch in t.chars() {
                        if ch.is_whitespace() {
                            if !cur.is_empty() {
                                push(&mut words, std::mem::take(&mut cur), false, space);
                            }
                            space = true;
                        } else {
                            cur.push(ch);
                        }
                    }
                    if !cur.is_empty() {
                        push(&mut words, cur, false, space);
                        space = false;
                    }
                }
                Piece::Math(m) => {
                    push(&mut words, m, true, space);
                    space = false;
                }
            }
        }
    }
    let mut words = explode_math_words(words);
    repair_line_breaks(&mut words);
    join_math(&mut words);
    if !is_prose(&words) {
        return None;
    }
    Some(render(&words, math))
}

/// Strip a single-column `array` wrapper and split into visual rows.
fn rows(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    let body = match s.strip_prefix("\\begin{array}{") {
        Some(rest) => {
            let close = rest.find('}')?;
            if !matches!(&rest[..close], "l" | "c" | "r") {
                return None; // several columns: a table, not a paragraph
            }
            rest[close + 1..].strip_suffix("\\end{array}")?.trim()
        }
        None => s,
    };
    let b = body.as_bytes();
    let (mut rows, mut depth, mut start, mut i) = (Vec::new(), 0i32, 0, 0);
    while i < b.len() {
        match b[i] {
            b'\\' if b.get(i + 1) == Some(&b'\\') && depth == 0 => {
                rows.push(&body[start..i]);
                i += 2;
                start = i;
                continue;
            }
            b'\\' => i += 1,
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    rows.push(&body[start..]);
    let rows: Vec<String> = rows
        .into_iter()
        .map(|r| {
            let mut r = r.trim();
            if let Some(rest) = r.strip_prefix('[').and_then(|x| x.split_once(']')) {
                // `\\[-2pt]`: a spacing argument on the row break, not content.
                let len = rest.0.trim_start_matches('-');
                if !len.is_empty()
                    && len.ends_with(|c: char| c.is_ascii_alphabetic())
                    && len.chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    r = rest.1.trim();
                }
            }
            while r.starts_with('{') && matching_brace(r, 0) == Some(r.len() - 1) {
                r = r[1..r.len() - 1].trim();
            }
            r.to_string()
        })
        .filter(|r| !r.is_empty())
        .collect();
    (!rows.is_empty()).then_some(rows)
}

/// Name and byte length of the control sequence starting at `s[i]` (a backslash).
fn control(s: &str, i: usize) -> (&str, usize) {
    let rest = &s[i + 1..];
    let n = match rest.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len()),
        Some(c) => c.len_utf8(),
        None => 0,
    };
    (&s[i..i + 1 + n], 1 + n)
}

/// Index just past a command's name and the arguments that belong to it (`{..}`, `^x`, `_{..}`).
fn command_end(s: &str, mut j: usize) -> usize {
    let b = s.as_bytes();
    loop {
        match b.get(j) {
            Some(b'{') => match matching_brace(s, j) {
                Some(e) => j = e + 1,
                None => return s.len(),
            },
            Some(b'^') | Some(b'_') => {
                j += 1;
                j = arg_end(s, j);
            }
            _ => return j,
        }
    }
}

/// Index just past the single argument starting at `s[j]` (a group, a command or one character).
fn arg_end(s: &str, j: usize) -> usize {
    match s.as_bytes().get(j) {
        Some(b'{') => matching_brace(s, j).map_or(s.len(), |e| e + 1),
        Some(b'\\') => j + control(s, j).1,
        Some(_) => j + s[j..].chars().next().map_or(0, char::len_utf8),
        None => j,
    }
}

fn push_text(out: &mut Vec<Piece>, t: &str) {
    match out.last_mut() {
        Some(Piece::Text(prev)) => prev.push_str(t),
        _ => out.push(Piece::Text(t.to_string())),
    }
}

/// Emit a run of characters found outside any text group: digits are text, the rest is math,
/// and trailing sentence punctuation always goes back to being text.
fn flush(run: &mut String, out: &mut Vec<Piece>) {
    if run.is_empty() {
        return;
    }
    let r = std::mem::take(run);
    let body = r.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    if !body.is_empty() {
        if body
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
        {
            push_text(out, body);
        } else {
            out.push(Piece::Math(body.to_string()));
        }
    }
    push_text(out, &r[body.len()..]);
}

/// Split one row into text and math pieces.
fn scan(s: &str, in_text: bool, out: &mut Vec<Piece>) {
    let b = s.as_bytes();
    let (mut i, mut run) = (0, String::new());
    while i < s.len() {
        match b[i] {
            b'~' => {
                flush(&mut run, out);
                push_text(out, " ");
                i += 1;
            }
            b'{' => {
                let end = matching_brace(s, i).unwrap_or(s.len() - 1);
                flush(&mut run, out);
                scan(&s[i + 1..end], in_text, out);
                i = end + 1;
            }
            b'}' => i += 1,
            b'\\' => {
                let (name, len) = control(s, i);
                match name {
                    "\\mathrm" | "\\text" | "\\textrm" | "\\mbox" | "\\mathit" | "\\textit"
                    | "\\textup" | "\\textnormal" | "\\textsf" | "\\textbf" | "\\textsc"
                        if b.get(i + len) == Some(&b'{') =>
                    {
                        let open = i + len;
                        let end = matching_brace(s, open).unwrap_or(s.len() - 1);
                        flush(&mut run, out);
                        // A new text group after unspaced text is a new word ("5.7" + "Let").
                        if let Some(Piece::Text(t)) = out.last()
                            && !t.is_empty()
                            && !t.ends_with(char::is_whitespace)
                        {
                            push_text(out, " ");
                        }
                        scan(&s[open + 1..end], true, out);
                        i = end + 1;
                    }
                    "\\;" | "\\," | "\\:" | "\\ " | "\\>" | "\\quad" | "\\qquad" => {
                        flush(&mut run, out);
                        push_text(out, " ");
                        i += len;
                    }
                    "\\!" => i += len,
                    // Layout the model sometimes invents at a line break: not part of the text.
                    "\\vspace" | "\\hspace" | "\\vskip" | "\\hskip"
                        if b.get(i + len) == Some(&b'{') =>
                    {
                        i = matching_brace(s, i + len).map_or(s.len(), |e| e + 1);
                    }
                    "\\hfill" | "\\noindent" | "\\newline" | "\\par" => i += len,
                    _ => {
                        let end = command_end(s, i + len);
                        if in_text {
                            flush(&mut run, out);
                            out.push(Piece::Math(s[i..end].to_string()));
                        } else {
                            run.push_str(&s[i..end]);
                        }
                        i = end;
                    }
                }
            }
            // Inside a text group an exponent still has to keep its braces: `e^{i\pi}`, not `e^i\pi`.
            c @ (b'^' | b'_') if in_text => {
                push_text(out, if c == b'^' { "^" } else { "_" });
                let end = arg_end(s, i + 1);
                push_text(out, &s[i + 1..end]);
                i = end;
            }
            c @ (b'^' | b'_') if !in_text => {
                run.push(c as char);
                let end = arg_end(s, i + 1);
                run.push_str(&s[i + 1..end]);
                i = end;
            }
            _ => {
                let ch = s[i..].chars().next().unwrap();
                if in_text {
                    push_text(out, ch.encode_utf8(&mut [0; 4]));
                } else {
                    run.push(ch);
                }
                i += ch.len_utf8();
            }
        }
    }
    flush(&mut run, out);
}

/// Wrap the words of running text that are math (lone letters, intervals, expressions) in `$...$`,
/// leaving anything already inside `$...$` alone. Whitespace is preserved. This is for text that
/// was read by a model which writes plain letters for variables, using the same word rules as
/// prose mode.
pub fn mark_math(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for (i, part) in text.split('$').enumerate() {
        if i > 0 {
            out.push('$');
        }
        if i % 2 == 1 {
            out.push_str(part); // already math
            continue;
        }
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String| {
            if word.is_empty() {
                return;
            }
            let (lead, core, trail) = split_punct(word);
            if !core.is_empty() && looks_math(core) {
                out.push_str(&format!("{lead}${core}${trail}"));
            } else {
                out.push_str(word);
            }
            word.clear();
        };
        for ch in part.chars() {
            if ch.is_whitespace() {
                flush(&mut word, &mut out);
                out.push(ch);
            } else {
                word.push(ch);
            }
        }
        flush(&mut word, &mut out);
    }
    out
}

/// The LaTeX and TeX logos are drawn, not typed, so the model reads them as plain capitals
/// (`LATEX`). Outside math, turn them back into the macros: `LATEX` or `LaTeX` becomes `\LaTeX{}`,
/// `TEX` or `TeX` becomes `\TeX{}`. (The `{}` keeps the space after the macro.)
pub fn replace_logos(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for (i, part) in text.split('$').enumerate() {
        if i > 0 {
            out.push('$');
        }
        if i % 2 == 1 {
            out.push_str(part); // math: leave alone
            continue;
        }
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String| {
            out.push_str(match word.as_str() {
                "LATEX" | "LaTeX" | "LATeX" => "\\LaTeX{}",
                "TEX" | "TeX" => "\\TeX{}",
                other => other,
            });
            word.clear();
        };
        for ch in part.chars() {
            if ch.is_ascii_alphanumeric() {
                word.push(ch);
            } else {
                flush(&mut word, &mut out);
                out.push(ch);
            }
        }
        flush(&mut word, &mut out);
    }
    out
}

/// Did the model lose the spaces between words? True if running text (outside `$...$`) has a
/// word of 20 or more letters in a row, which no ordinary English word reaches.
pub fn has_run_together_words(prose: &str) -> bool {
    prose.split('$').step_by(2).any(|text| {
        text.split(|c: char| !c.is_ascii_alphabetic())
            .any(|w| w.len() >= 20)
    })
}

const RELATIONS: &[&str] = &[
    "\\leq",
    "\\geq",
    "\\neq",
    "\\le",
    "\\ge",
    "\\in",
    "\\notin",
    "\\subset",
    "\\subseteq",
    "\\to",
    "\\approx",
    "\\equiv",
    "\\cdot",
    "\\times",
    "\\pm",
    "\\cup",
    "\\cap",
    "\\mid",
    "\\ne",
    "\\cdots",
    "\\ldots",
];

/// Does this math end in something that must be followed by more math (`+`, `=`, `\\geq`...)?
fn ends_open(m: &str) -> bool {
    m.ends_with(['+', '-', '=', '<', '>', '*', '/']) || RELATIONS.iter().any(|r| m.ends_with(r))
}

/// Does this math start with something that must follow other math (`=`, `\\leq`, `+`...)?
fn starts_open(m: &str) -> bool {
    m.starts_with(['+', '-', '=', '<', '>'])
        || RELATIONS.iter().any(|r| {
            m.strip_prefix(r)
                .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphabetic()))
        })
}

fn is_number(w: &Word) -> bool {
    !w.math && !w.text.is_empty() && w.text.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Words of running text that are really math (`f`, `[a,b]`, `n-1`) become math words, with any
/// punctuation around them split off as its own word.
fn explode_math_words(words: Vec<Word>) -> Vec<Word> {
    let mut out = Vec::with_capacity(words.len());
    for w in words {
        if w.math {
            out.push(w);
            continue;
        }
        let (lead, core, trail) = split_punct(&w.text);
        if core.is_empty() || !looks_math(core) {
            out.push(w);
            continue;
        }
        let mk = |text: &str, math: bool, space_before: bool, row_start: bool| Word {
            text: text.into(),
            math,
            space_before,
            row_start,
        };
        let has_lead = !lead.is_empty();
        if has_lead {
            out.push(mk(lead, false, w.space_before, w.row_start));
        }
        out.push(mk(
            core,
            true,
            if has_lead { false } else { w.space_before },
            w.row_start && !has_lead,
        ));
        if !trail.is_empty() {
            out.push(mk(trail, false, false, false));
        }
    }
    out
}

/// Repair what a visual line break did to a word: "cut-" + "vertex" is one word.
fn repair_line_breaks(words: &mut [Word]) {
    for i in 1..words.len() {
        if words[i].row_start
            && !words[i - 1].math
            && !words[i].math
            && words[i - 1].text.ends_with('-')
            && words[i].text.starts_with(|c: char| c.is_ascii_lowercase())
        {
            words[i].space_before = false;
        }
    }
}

/// Merge neighbouring math into one formula: pieces touching each other, an operator and the
/// operand after it (`n\\geq` `1`), and a formula cut by a line break (`1+2+` `\\cdots+n`).
fn join_math(words: &mut Vec<Word>) {
    let mut i = 1;
    while i < words.len() {
        let (prev, next) = (&words[i - 1], &words[i]);
        let merge = prev.math
            && ((next.math
                && (!next.space_before || ends_open(&prev.text) || starts_open(&next.text)))
                || (is_number(next) && ends_open(&prev.text)));
        if merge {
            let next = words.remove(i);
            words[i - 1].text.push_str(&next.text);
        } else {
            i += 1;
        }
    }
}

const TRAILING: &[char] = &[',', '.', ';', ':', '!', '?', ')', '"', '\'', '”', '’'];
const LEADING: &[char] = &['(', '"', '\'', '“', '‘'];

fn split_punct(w: &str) -> (&str, &str, &str) {
    let core_start = w.len() - w.trim_start_matches(LEADING).len();
    let core_end = core_start + w[core_start..].trim_end_matches(TRAILING).len();
    (&w[..core_start], &w[core_start..core_end], &w[core_end..])
}

fn has_real_word(s: &str) -> bool {
    s.as_bytes()
        .windows(3)
        .any(|w| w.iter().all(u8::is_ascii_alphabetic))
}

/// Should this word of running text be typeset as math?
fn looks_math(core: &str) -> bool {
    if core.len() == 1 {
        let c = core.as_bytes()[0];
        return c.is_ascii_alphabetic() && !matches!(c, b'a' | b'A' | b'I');
    }
    // [12] is a reference label
    if core.len() >= 3
        && core.starts_with('[')
        && core.ends_with(']')
        && core[1..core.len() - 1].chars().all(|c| c.is_ascii_digit())
    {
        return false;
    }
    // my_name is a file name or identifier written in text, not a subscript
    if let Some((a, b)) = core.split_once('_')
        && a.len() >= 2
        && b.len() >= 2
        && a.chars().all(|c| c.is_ascii_alphabetic())
        && b.chars().all(|c| c.is_ascii_alphabetic())
    {
        return false;
    }
    core.contains(['=', '<', '>', '+', '^', '_', '[', ']', '\\'])
        || (core.chars().any(|c| c.is_ascii_digit())
            && core.chars().any(|c| c.is_ascii_alphabetic())
            && !has_real_word(core))
}

fn is_punct_only(w: &Word) -> bool {
    !w.math && w.text.chars().all(|c| TRAILING.contains(&c))
}

/// A sentence: at least four real words, and at least as many word-like tokens as math-like ones.
/// (Counting only long words against everything would punish short ones like "if", "is", "on".)
fn is_prose(words: &[Word]) -> bool {
    let (mut real, mut wordy, mut mathy) = (0, 0, 0);
    for w in words.iter().filter(|w| !is_punct_only(w)) {
        let math_like = w.math || {
            let (_, core, _) = split_punct(&w.text);
            looks_math(core)
        };
        if math_like {
            mathy += 1;
        } else {
            wordy += 1;
            real += usize::from(has_real_word(&w.text));
        }
    }
    real >= 4 && wordy >= mathy
}

fn render(words: &[Word], math: &dyn Fn(&str) -> String) -> String {
    let mut out = String::new();
    let mut prev: Option<&Word> = None;
    for w in words {
        let space = match prev {
            None => false,
            Some(_) if w.space_before => !is_punct_only(w) && !out.ends_with(['(', '[']),
            // No space in the model's output. Between a word and a formula there is still one in
            // running text, unless the text side is punctuation or a hyphen: `$x$-axis`, `$G$.`
            Some(p) if p.math != w.math => {
                if w.math {
                    !p.text.ends_with(['(', '[', '-', '\'', '"'])
                } else {
                    !w.text
                        .starts_with(['-', '\'', '.', ',', ';', ':', ')', '!', '?', '"'])
                }
            }
            Some(_) => false,
        };
        if space {
            out.push(' ');
        }
        if w.math {
            out.push_str(&format!("${}$", math(&w.text)));
        } else {
            out.push_str(&w.text);
        }
        prev = Some(w);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_logos_become_macros_outside_math_only() {
        use super::replace_logos;
        assert_eq!(
            replace_logos("I am learning LATEX. LATEX is great."),
            "I am learning \\LaTeX{}. \\LaTeX{} is great."
        );
        assert_eq!(
            replace_logos("plain TeX and LaTeX"),
            "plain \\TeX{} and \\LaTeX{}"
        );
        // inside math, and inside longer words, nothing changes
        assert_eq!(replace_logos("let $TEX$ be"), "let $TEX$ be");
        assert_eq!(replace_logos("TEXT and LATEXED"), "TEXT and LATEXED");
    }

    use crate::tidy::Kind;
    use crate::tidy::tidy_with_kind;

    // Inputs are real model output for textbook-style sentences.
    fn prose(raw: &str) -> String {
        let (text, kind) = tidy_with_kind(raw);
        assert_eq!(kind, Kind::Prose, "not recognised as prose: {text}");
        text
    }

    #[test]
    fn two_row_array_with_inline_math() {
        // The textbook screenshot this feature was built for.
        assert_eq!(
            prose(
                r"\begin{array} { c } { 5 . 4 \mathrm { ~ P r o v e ~ t h a t ~ i f ~ v ~ i s ~ a ~ c u t - v e r t e x ~ o f ~ a ~ g r a p h ~ G ~ , ~ t h e n ~ v ~ i s ~ n o t ~ a ~ c u t - v e r t e x } } \\ { \mathrm { ~ o f ~ t h e ~ c o m p l e m e n t ~ \overline { { G } } ~ o f ~ G ~ . } } \end{array}"
            ),
            r"5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$."
        );
    }

    #[test]
    fn one_mathrm_group_with_bracketed_interval() {
        assert_eq!(
            prose(
                r"\mathrm { I f ~ f ~ i s ~ c o n t i n u o u s ~ o n ~ [ a , b ] ~ , ~ t h e n ~ f ~ a t t a i n s ~ a ~ m a x i m u m ~ v a l u e ~ o n ~ [ a , b ] ~ . }"
            ),
            r"If $f$ is continuous on $[a,b]$, then $f$ attains a maximum value on $[a,b]$."
        );
    }

    #[test]
    fn math_already_outside_the_text_groups() {
        assert_eq!(
            prose(
                r"\mathrm { F i n d ~ a l l ~ r e a l ~ n u m b e r s ~ } x \; \mathrm { s u c h ~ t h a t } \; x ^ { 2 } - 5 x + 6 = 0 ."
            ),
            r"Find all real numbers $x$ such that $x^2-5x+6=0$."
        );
    }

    #[test]
    fn letters_and_digit_words_inside_text() {
        assert_eq!(
            prose(
                r"\mathrm { L e t ~ G ~ b e ~ a ~ c o n n e c t e d ~ g r a p h ~ w i t h ~ n ~ v e r t i c e s . ~ S h o w ~ t h a t ~ G ~ h a s ~ a t ~ l e a s t ~ n - 1 ~ e d g e s . }"
            ),
            r"Let $G$ be a connected graph with $n$ vertices. Show that $G$ has at least $n-1$ edges."
        );
    }

    #[test]
    fn textup_groups_are_text_and_glued_words_are_detected() {
        // Real shape of a three-line read: a different text command, and the spaces are gone.
        let out = prose(
            r"\begin{array}{l}{5.7\textup{Let}G\textup{beaconnectedgraphwith}n\textup{verticesandletvbeavertexofG.}} \\ {\textup{Provethat}G-\nu\textup{hasatmost}} \end{array}",
        );
        assert!(
            out.starts_with("5.7 Let $G$ beaconnectedgraphwith $n$"),
            "{out}"
        );
        assert!(super::has_run_together_words(&out));
        assert!(!super::has_run_together_words(
            "5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\\overline{G}$."
        ));
        // Punctuation and hyphens stay attached to the formula before them.
        assert_eq!(
            prose(r"\mathrm { T h e ~ x - a x i s ~ h a s ~ t w o ~ p o i n t s ~ } x ."),
            "The x-axis has two points $x$."
        );
    }

    #[test]
    fn mark_math_wraps_plain_variables_and_keeps_existing_math() {
        use super::mark_math;
        assert_eq!(
            mark_math(
                r"5.4 Prove that if v is a cut-vertex of a graph G, then v is not a cut-vertex of the complement $\overline{G}$ of G."
            ),
            r"5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$."
        );
        assert_eq!(
            mark_math("If f is continuous on [a,b], then f attains a maximum value."),
            "If $f$ is continuous on $[a,b]$, then $f$ attains a maximum value."
        );
        // Words, the article "a" and numbers are left alone; layout is preserved.
        assert_eq!(
            mark_math("Show that  a 3-regular graph has order 4."),
            "Show that  a 3-regular graph has order 4."
        );
    }

    #[test]
    fn exponent_inside_a_text_group_keeps_its_braces() {
        // Real output for "...Euler showed that $e^{i\pi}+1=0$,".
        assert_eq!(
            prose(
                r"\mathrm{The~quick~brown~fox~jumps~over~the~lazy~dog.~Euler~showed~that~e^{i\pi}+1=0~,}"
            ),
            r"The quick brown fox jumps over the lazy dog. Euler showed that $e^{i\pi}+1=0$,"
        );
    }

    #[test]
    fn operator_and_its_operands_become_one_formula() {
        // Here the model put n inside the text group but \geq and the 1 outside it.
        assert_eq!(
            prose(
                r"\mathrm { P r o v e ~ t h a t ~ f o r ~ e v e r y ~ i n t e g e r ~ n \geq ~ 1 ~ , ~ t h e ~ s u m ~ 1 + 2 + \cdots + n ~ e q u a l s ~ n ( n + 1 ) / 2 ~ . }"
            ),
            r"Prove that for every integer $n\geq1$, the sum $1+2+\cdots+n$ equals $n(n+1)/2$."
        );
    }

    #[test]
    fn layout_commands_at_a_line_break_are_dropped() {
        assert_eq!(
            prose(
                r"\begin{array} { l } { \mathrm { P r o v e ~ t h a t ~ f o r ~ e v e r y ~ i n t e g e r ~ } n \geq 1 , \mathrm { ~ t h e ~ s u m ~ } 1 + 2 + } \\ \vspace { - 0 . 2 5 i n } { \cdots + n \mathrm { ~ e q u a l s ~ } n ( n + 1 ) / 2 . } \end{array}"
            ),
            r"Prove that for every integer $n\geq1$, the sum $1+2+\cdots+n$ equals $n(n+1)/2$."
        );
        // A spacing argument on the row break, `\\[-2pt]`, is not content either.
        assert_eq!(
            prose(
                r"\begin{array} { l } { \mathrm { T h e ~ g r a p h ~ i s ~ a ~ t h r e e - } } \\ [ - 2 p t ] { \mathrm { r e g u l a r ~ g r a p h ~ h e r e } } \end{array}"
            ),
            "The graph is a three-regular graph here"
        );
    }

    #[test]
    fn formula_split_across_the_line_break_is_rejoined() {
        assert_eq!(
            prose(
                r"\begin{array} { l } { \mathrm { P r o v e ~ t h a t ~ f o r ~ e v e r y ~ i n t e g e r ~ } n \geq 1 , \mathrm { ~ t h e ~ s u m ~ } 1 + 2 + } \\ { \cdots + n \mathrm { ~ e q u a l s ~ } n ( n + 1 ) / 2 . } \end{array}"
            ),
            r"Prove that for every integer $n\geq1$, the sum $1+2+\cdots+n$ equals $n(n+1)/2$."
        );
    }

    #[test]
    fn misread_letters_are_kept_not_hidden() {
        // The model read an italic v as \nu here; we keep what it wrote rather than guess.
        let out = prose(
            r"\begin{array} { l } { 5 . 4 \mathrm { ~ P r o v e ~ t h a t ~ i f ~ \ v ~ i s ~ a ~ c u t - v e r t e x ~ o f ~ a ~ g r a p h ~ G ~ , ~ t h e n ~ \nu ~ } } \\ { \mathrm { i s ~ n o t ~ a ~ c u t - v e r t e x } } \\ \end{array}",
        );
        assert!(out.contains(r"$\nu$ is not"), "{out}");
    }

    #[test]
    fn words_broken_at_the_line_end_are_glued() {
        assert_eq!(
            prose(
                r"\begin{array} { l } { \mathrm { T h e ~ g r a p h ~ i s ~ a ~ t h r e e - } } \\ { \mathrm { r e g u l a r ~ g r a p h ~ h e r e } } \end{array}"
            ),
            "The graph is a three-regular graph here"
        );
    }

    #[test]
    fn formulas_stay_formulas() {
        // Little or no prose: must not be turned into a sentence.
        for raw in [
            r"x = \frac { - b \pm \sqrt { b ^ { 2 } - 4 a c } } { 2 a }",
            r"\text { f o r ~ a l l ~ } x \in \mathbb { R } , \quad f ( x ) \geq 0 \text { w h e n e v e r ~ } x > 0",
            r"f ( x ) = \begin{cases} { x ^ { 2 } } & { \mathrm { i f ~ } x \geq 0 } \\ { - x } & { \mathrm { o t h e r w i s e } } \end{cases}",
            r"\mathrm { V a r } ( X ) = \mathbb { E } [ X ^ { 2 } ] - ( \mathbb { E } [ X ] ) ^ { 2 }",
        ] {
            assert_eq!(tidy_with_kind(raw).1, Kind::Math, "{raw}");
        }
    }

    #[test]
    fn a_table_is_not_prose() {
        let raw = r"\begin{array} { | c | c | } \hline { \mathrm { N a m e } } & { \mathrm { A g e } } \\ \hline \end{array}";
        assert_eq!(tidy_with_kind(raw).1, Kind::Math);
    }
}
