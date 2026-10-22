//! turns the vision model's markdown into LaTeX that renders the same.

use crate::{prose, table, tidy, vlm};

/// a literal $ in text, hidden while we split on $ and put back as \$.
const DOLLAR: char = '\u{E000}';
/// brackets of inline math indexes, parked while we split into blocks.
const OPEN: char = '\u{E001}';
const CLOSE: char = '\u{E002}';

enum Piece {
    /// a paragraph of text.
    Para(String),
    /// anything on its own lines, heading, list, equation or code.
    Raw(String),
}

/// the model's text without tables, as LaTeX.
pub fn to_latex(raw: &str) -> String {
    let segs = vlm::segments(raw);
    let mut pieces: Vec<Piece> = Vec::new();
    let mut inline: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut skip = 0usize; // bytes of the next text segment already used, like an equation number.
    let mut had_display = false;
    for (i, seg) in segs.iter().enumerate() {
        if !seg.math {
            let body = &seg.body[skip.min(seg.body.len())..];
            skip = 0;
            buf.push_str(&body.replace('$', &DOLLAR.to_string()));
        } else if !seg.display {
            buf.push(OPEN);
            buf.push_str(&inline.len().to_string());
            buf.push(CLOSE);
            inline.push(tidy::tidy_formula(&seg.body));
        } else {
            flush_text(&mut buf, &inline, &mut pieces);
            let (body, tagged) = strip_tag(&seg.body);
            let number = segs
                .get(i + 1)
                .filter(|next| !next.math)
                .and_then(|next| leading_equation_number(&next.body));
            if let Some((_, used)) = &number {
                skip = *used;
            }
            let body = tidy::tidy_formula(&body);
            pieces.push(Piece::Raw(display(&body, tagged || number.is_some())));
            had_display = true;
        }
    }
    flush_text(&mut buf, &inline, &mut pieces);

    // a bare number after a display is the page number.
    if had_display
        && pieces.len() >= 2
        && matches!(pieces.last(), Some(Piece::Para(t)) if t.len() <= 4 && t.chars().all(|c| c.is_ascii_digit()))
    {
        pieces.pop();
    }

    // a proof's end mark goes flush right.
    let mut merged: Vec<Piece> = Vec::new();
    for piece in pieces {
        match (piece, merged.last_mut()) {
            (Piece::Para(t), Some(Piece::Para(prev))) if is_qed(&t) => {
                prev.push_str(" \\hfill ");
                prev.push_str(&t);
            }
            (piece, _) => merged.push(piece),
        }
    }
    let pieces = merged;

    // paragraphs get a blank line, everything else gets its own line.
    let mut out = String::new();
    let mut prev_was_para = false;
    for piece in pieces {
        let (text, is_para) = match piece {
            Piece::Para(t) => (t, true),
            Piece::Raw(t) => (t, false),
        };
        if !out.is_empty() {
            out.push_str(if prev_was_para && is_para {
                "\n\n"
            } else {
                "\n"
            });
        }
        out.push_str(&text);
        prev_was_para = is_para;
    }
    out
}

fn is_qed(text: &str) -> bool {
    matches!(text, "$\\square$" | "$\\blacksquare$")
}

/// a table cell, cleaned like a paragraph with no block structure.
pub fn cell(raw: &str, header: bool) -> String {
    if is_body_label(raw, header) {
        return raw.trim().to_string();
    }
    inline(raw)
}

fn is_body_label(raw: &str, header: bool) -> bool {
    let t = raw.trim();
    !header && t.chars().count() == 1 && t.chars().all(|c| c.is_ascii_alphabetic())
}

fn inline(raw: &str) -> String {
    let (mut parked, mut buf) = (Vec::new(), String::new());
    for seg in vlm::segments(raw) {
        if seg.math {
            buf.push(OPEN);
            buf.push_str(&parked.len().to_string());
            buf.push(CLOSE);
            parked.push(tidy::tidy_formula(&seg.body));
        } else {
            buf.push_str(&seg.body.replace('$', &DOLLAR.to_string()));
        }
    }
    finish(&buf, &parked)
}

fn flush_text(buf: &mut String, inline: &[String], pieces: &mut Vec<Piece>) {
    if !buf.trim().is_empty() {
        pieces.extend(blocks(buf, inline));
    }
    buf.clear();
}

// display equations.

/// (1), (2.3) or (4a) at the start is an equation number.
pub fn leading_equation_number(text: &str) -> Option<(String, usize)> {
    let trimmed = text.trim_start();
    let inner = trimmed.strip_prefix('(')?;
    let end = inner.find(')')?;
    let number = &inner[..end];
    let valid = !number.is_empty()
        && number.len() <= 8
        && number.starts_with(|c: char| c.is_ascii_digit())
        && number
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.');
    valid.then(|| (number.to_string(), text.len() - trimmed.len() + end + 2))
}

/// drop a \tag{..} from display math.
pub fn strip_tag(body: &str) -> (String, bool) {
    if let Some(start) = body.find("\\tag{")
        && let Some(len) = body[start..].find('}')
    {
        let cleaned = format!("{}{}", &body[..start], &body[start + len + 1..]);
        return (cleaned.trim().to_string(), true);
    }
    (body.to_string(), false)
}

/// environments that are display math already.
const OWN_DISPLAY: [&str; 8] = [
    "align", "alignat", "flalign", "gather", "multline", "eqnarray", "equation", "split",
];

fn is_own_display(body: &str) -> bool {
    let b = body.trim_start();
    b.strip_prefix("\\begin{").is_some_and(|rest| {
        let name = rest.split('}').next().unwrap_or("").trim_end_matches('*');
        OWN_DISPLAY.contains(&name) && name != "split"
    })
}

fn display(body: &str, numbered: bool) -> String {
    if is_own_display(body) {
        body.to_string()
    } else if numbered {
        format!("\\begin{{equation}}\n{body}\n\\end{{equation}}")
    } else {
        format!("\\[\n{body}\n\\]")
    }
}

// headings, lists, code and paragraphs.

struct Item {
    /// the bullet, or # for numbered.
    key: char,
    indent: usize,
    number: Option<usize>,
    text: String,
    /// the line as the model wrote it.
    original: String,
}

fn list_marker(line: &str) -> Option<Item> {
    let indent = line.len() - line.trim_start().len();
    let t = line.trim_start();
    let c = t.chars().next()?;
    if matches!(c, '-' | '*' | '+' | '•' | '·' | '–' | '◦' | '▪') {
        let rest = &t[c.len_utf8()..];
        if rest.starts_with([' ', '\t']) {
            return Some(Item {
                key: c,
                indent,
                number: None,
                text: rest.trim().to_string(),
                original: line.trim().to_string(),
            });
        }
    }
    let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
    if (1..=3).contains(&digits.len()) {
        let rest = &t[digits.len()..];
        if rest.starts_with(". ") || rest.starts_with(") ") {
            return Some(Item {
                key: '#',
                indent,
                number: digits.parse().ok(),
                text: rest[2..].trim().to_string(),
                original: line.trim().to_string(),
            });
        }
    }
    None
}

/// # Title and friends become \section*{..}.
fn heading(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.chars().take_while(|&c| c == '#').count();
    let rest = t[level..].strip_prefix(' ')?;
    (1..=6).contains(&level).then_some((level, rest.trim()))
}

/// a block before cleanup, paragraphs wait as raw lines.
enum Draft {
    Lines(Vec<String>),
    Done(Piece),
}

fn blocks(text: &str, inline: &[String]) -> Vec<Piece> {
    let mut drafts: Vec<Draft> = Vec::new();
    let (mut para, mut list): (Vec<String>, Vec<Item>) = (Vec::new(), Vec::new());
    let mut code: Option<Vec<String>> = None;
    let mut after_blank = false;

    let flush_para = |para: &mut Vec<String>, drafts: &mut Vec<Draft>| {
        if !para.is_empty() {
            drafts.push(Draft::Lines(std::mem::take(para)));
        }
    };
    let flush_list = |list: &mut Vec<Item>, drafts: &mut Vec<Draft>| {
        if list.is_empty() {
            return;
        }
        let items = std::mem::take(list);
        // a lone 5. is a problem number, so no list.
        if items.len() == 1 && items[0].number.is_some() {
            drafts.push(Draft::Lines(vec![items[0].original.clone()]));
        } else {
            drafts.push(Draft::Done(Piece::Raw(list_latex(&items, inline))));
        }
    };
    let verbatim = |lines: &[String]| {
        Draft::Done(Piece::Raw(format!(
            "\\begin{{verbatim}}\n{}\n\\end{{verbatim}}",
            lines.join("\n")
        )))
    };

    for line in text.lines() {
        if let Some(lines) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                drafts.push(verbatim(lines));
                code = None;
            } else {
                lines.push(line.to_string());
            }
            continue;
        }
        if line.trim().is_empty() {
            flush_para(&mut para, &mut drafts);
            after_blank = true;
            continue;
        }
        if line.trim_start().starts_with("```") {
            flush_para(&mut para, &mut drafts);
            flush_list(&mut list, &mut drafts);
            code = Some(Vec::new());
        } else if let Some((level, title)) = heading(line) {
            flush_para(&mut para, &mut drafts);
            flush_list(&mut list, &mut drafts);
            let cmd = match level {
                1 | 2 => "section*",
                3 => "subsection*",
                _ => "subsubsection*",
            };
            drafts.push(Draft::Done(Piece::Raw(format!(
                "\\{cmd}{{{}}}",
                finish(title, inline)
            ))));
        } else if let Some(item) = list_marker(line) {
            flush_para(&mut para, &mut drafts);
            list.push(item);
        } else if !list.is_empty() && !after_blank {
            // a wrapped line of the last list item.
            let last = list.last_mut().expect("checked non-empty");
            last.text.push(' ');
            last.text.push_str(line.trim());
            last.original.push(' ');
            last.original.push_str(line.trim());
        } else {
            flush_list(&mut list, &mut drafts);
            para.push(line.trim().to_string());
        }
        after_blank = false;
    }
    if let Some(lines) = code {
        drafts.push(verbatim(&lines));
    }
    flush_para(&mut para, &mut drafts);
    flush_list(&mut list, &mut drafts);

    // check consecutive paragraphs together for plain text tables.
    let mut pieces = Vec::new();
    let mut group: Vec<Vec<String>> = Vec::new();
    for draft in drafts {
        match draft {
            Draft::Lines(lines) => group.push(lines),
            Draft::Done(piece) => {
                resolve_paragraphs(&mut group, inline, &mut pieces);
                pieces.push(piece);
            }
        }
    }
    resolve_paragraphs(&mut group, inline, &mut pieces);
    pieces
}

/// how many lines from the start are a plain text table.
fn plain_table_len(lines: &[&str]) -> usize {
    // a line ending in a colon introduces what follows.
    if lines.first().is_none_or(|l| l.trim_end().ends_with(':')) {
        return 0;
    }
    let width = lines.first().map_or(0, |l| l.split_whitespace().count());
    if !(2..=10).contains(&width) {
        return 0;
    }
    let rows: Vec<Vec<&str>> = lines
        .iter()
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .take_while(|r| r.len() == width && r.iter().all(|w| w.chars().count() <= 16))
        .collect();
    let numeric_column =
        (0..width).any(|c| rows.iter().skip(1).all(|r| table::is_number_like(r[c])));
    // two columns only count as a table when the labels differ.
    let labels_differ = width > 2 || {
        let mut firsts: Vec<&str> = rows.iter().map(|r| r[0]).collect();
        firsts.sort_unstable();
        firsts.dedup();
        firsts.len() == rows.len()
    };
    if rows.len() >= 3 && numeric_column && labels_differ {
        rows.len()
    } else {
        0
    }
}

/// clean the paragraphs in a group, turning plain text tables into tabulars.
fn resolve_paragraphs(group: &mut Vec<Vec<String>>, inline: &[String], pieces: &mut Vec<Piece>) {
    let flat: Vec<(usize, &str)> = group
        .iter()
        .enumerate()
        .flat_map(|(i, p)| p.iter().map(move |l| (i, l.as_str())))
        .collect();
    let texts: Vec<&str> = flat.iter().map(|(_, l)| *l).collect();
    let mut plain_from = 0;
    let mut i = 0;
    let flush_plain = |from: usize, to: usize, pieces: &mut Vec<Piece>| {
        let mut k = from;
        while k < to {
            let block = flat[k].0;
            let end = (k..to).find(|&j| flat[j].0 != block).unwrap_or(to);
            let joined = flat[k..end]
                .iter()
                .map(|(_, l)| *l)
                .collect::<Vec<_>>()
                .join(" ");
            pieces.push(Piece::Para(finish(&joined, inline)));
            k = end;
        }
    };
    while i < flat.len() {
        let n = plain_table_len(&texts[i..]);
        if n == 0 {
            i += 1;
            continue;
        }
        flush_plain(plain_from, i, pieces);
        let rows: Vec<Vec<String>> = texts[i..i + n]
            .iter()
            .map(|l| l.split_whitespace().map(str::to_string).collect())
            .collect();
        let clean = |t: &str, header: bool| {
            if is_body_label(t, header) {
                t.trim().to_string()
            } else {
                finish(t, inline) // inline holds the math parked in the text.
            }
        };
        match table::from_rows(&rows, &clean) {
            Some(latex) => pieces.push(Piece::Raw(latex)),
            None => flush_plain(i, i + n, pieces),
        }
        i += n;
        plain_from = i;
    }
    flush_plain(plain_from, flat.len(), pieces);
    group.clear();
}

/// itemize and enumerate, nested by marker and indent.
fn list_latex(items: &[Item], inline: &[String]) -> String {
    struct Level {
        key: char,
        indent: usize,
        env: &'static str,
    }
    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<Level> = Vec::new();
    let pad = |depth: usize| "  ".repeat(depth);
    let close = |stack: &mut Vec<Level>, out: &mut Vec<String>| {
        if let Some(level) = stack.pop() {
            out.push(format!("{}\\end{{{}}}", pad(stack.len()), level.env));
        }
    };
    for item in items {
        while stack.last().is_some_and(|l| l.indent > item.indent) {
            close(&mut stack, &mut out);
        }
        let same = stack.last().is_some_and(|l| l.key == item.key);
        if !same {
            // back to a level we opened, or a new deeper one.
            if let Some(pos) = stack.iter().rposition(|l| l.key == item.key) {
                while stack.len() > pos + 1 {
                    close(&mut stack, &mut out);
                }
            } else {
                let env = if item.number.is_some() {
                    "enumerate"
                } else {
                    "itemize"
                };
                out.push(format!("{}\\begin{{{env}}}", pad(stack.len())));
                if let Some(n) = item.number.filter(|&n| n != 1) {
                    let counter = ["enumi", "enumii", "enumiii", "enumiv"][stack.len().min(3)];
                    out.push(format!(
                        "{}\\setcounter{{{counter}}}{{{}}}",
                        pad(stack.len() + 1),
                        n - 1
                    ));
                }
                stack.push(Level {
                    key: item.key,
                    indent: item.indent,
                    env,
                });
            }
        }
        out.push(format!(
            "{}\\item {}",
            pad(stack.len()),
            finish(&item.text, inline)
        ));
    }
    while !stack.is_empty() {
        close(&mut stack, &mut out);
    }
    out.join("\n")
}

// text, one cleanup for paragraphs, list items and headings.

/// put the parked math back as $..$.
fn restore(text: &str, inline: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != OPEN {
            out.push(c);
            continue;
        }
        let index: String = chars.by_ref().take_while(|&c| c != CLOSE).collect();
        let body = index.parse::<usize>().ok().and_then(|i| inline.get(i));
        out.push('$');
        out.push_str(body.map_or("", |b| b.trim()));
        out.push('$');
    }
    out
}

/// apply f to the parts outside $..$.
fn outside_math(text: &str, f: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, part) in text.split('$').enumerate() {
        if i > 0 {
            out.push('$');
        }
        if i % 2 == 1 {
            out.push_str(part);
        } else {
            out.push_str(&f(part));
        }
    }
    out
}

/// quotes, dashes and symbols pdflatex can't read.
fn typography(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev: Option<char> = None;
    for c in text.chars() {
        match c {
            '“' => out.push_str("``"),
            '”' => out.push_str("''"),
            '‘' => out.push('`'),
            '"' => {
                let opening = prev.is_none_or(|p| p.is_whitespace() || "([{—–".contains(p));
                out.push_str(if opening { "``" } else { "''" });
            }
            '—' => out.push_str("---"),
            '–' => out.push_str("--"),
            '□' => out.push_str("$\\square$"),
            '∎' | '■' => out.push_str("$\\blacksquare$"),
            '×' => out.push_str("$\\times$"),
            '±' => out.push_str("$\\pm$"),
            '≤' => out.push_str("$\\leq$"),
            '≥' => out.push_str("$\\geq$"),
            '≠' => out.push_str("$\\neq$"),
            '≈' => out.push_str("$\\approx$"),
            '→' => out.push_str("$\\to$"),
            '←' => out.push_str("$\\leftarrow$"),
            '⇒' => out.push_str("$\\Rightarrow$"),
            '∈' => out.push_str("$\\in$"),
            '∞' => out.push_str("$\\infty$"),
            '∅' => out.push_str("$\\emptyset$"),
            '∪' => out.push_str("$\\cup$"),
            '∩' => out.push_str("$\\cap$"),
            '°' => out.push_str("$^\\circ$"),
            _ => out.push(c),
        }
        prev = Some(c);
    }
    out
}

fn escape_specials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '&' | '%' | '#' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// **bold** and *italic* as \textbf{} and \emph{}.
fn emphasis(text: &str) -> String {
    fn wrap(text: &str, marker: &str, cmd: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(marker) {
            let after = &rest[start + marker.len()..];
            let closes = after.find(marker).filter(|&end| {
                end > 0
                    && !after[..end].starts_with(char::is_whitespace)
                    && !after[..end].ends_with(char::is_whitespace)
                    && !after[..end].contains('\n')
            });
            match closes {
                Some(end) => {
                    out.push_str(&rest[..start]);
                    out.push_str(&format!("\\{cmd}{{{}}}", &after[..end]));
                    rest = &after[end + marker.len()..];
                }
                None => {
                    out.push_str(&rest[..start + marker.len()]);
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
    wrap(&wrap(text, "**", "textbf"), "*", "emph")
}

/// a paragraph, list item or heading as LaTeX text.
fn finish(text: &str, inline: &[String]) -> String {
    let marked = prose::mark_math(&restore(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        inline,
    ));
    let cleaned = outside_math(&marked, |t| {
        escape_specials(&typography(&prose::replace_logos(&emphasis(t))))
    });
    let cleaned = cleaned.replace(DOLLAR, "\\$");
    // an end mark at the end of a paragraph goes flush right.
    for mark in ["$\\square$", "$\\blacksquare$"] {
        if let Some(head) = cleaned.strip_suffix(mark)
            && head.ends_with(' ')
            && !head.trim().is_empty()
        {
            return format!("{}\\hfill {mark}", head);
        }
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_become_starred_sections() {
        assert_eq!(
            to_latex(
                "## Introduction\n\nThis document is short.\n\n### Background\n\nMore follows."
            ),
            "\\section*{Introduction}\nThis document is short.\n\\subsection*{Background}\nMore follows."
        );
    }

    #[test]
    fn bullets_and_numbers_become_lists_with_math_inside() {
        assert_eq!(
            to_latex("- First point\n\n- Second point with $x^{2}$\n\n- Third point"),
            "\\begin{itemize}\n  \\item First point\n  \\item Second point with $x^2$\n  \\item Third point\n\\end{itemize}"
        );
        assert_eq!(
            to_latex("1. Prove the claim.\n2. Show that $n \\geq 2$.\n3. Conclude."),
            "\\begin{enumerate}\n  \\item Prove the claim.\n  \\item Show that $n \\geq 2$.\n  \\item Conclude.\n\\end{enumerate}"
        );
    }

    #[test]
    fn a_different_bullet_nests_and_returning_to_the_first_closes_the_inner_list() {
        assert_eq!(
            to_latex("• Outer one\n\n– Inner two\n\n– Inner three\n\n• Outer four"),
            "\\begin{itemize}\n  \\item Outer one\n  \\begin{itemize}\n    \\item Inner two\n    \\item Inner three\n  \\end{itemize}\n  \\item Outer four\n\\end{itemize}"
        );
    }

    #[test]
    fn a_lone_problem_number_is_text_not_a_list() {
        assert_eq!(
            to_latex("5. Prove that $G$ is connected."),
            "5. Prove that $G$ is connected."
        );
        assert_eq!(
            to_latex("5.4 Prove that $G$ is connected."),
            "5.4 Prove that $G$ is connected."
        );
    }

    #[test]
    fn a_numbered_list_that_starts_late_keeps_its_numbers() {
        let latex = to_latex("3. Third thing.\n4. Fourth thing.");
        assert!(latex.contains("\\setcounter{enumi}{2}"), "{latex}");
    }

    #[test]
    fn paragraphs_keep_their_blank_line_but_wrapped_lines_join() {
        assert_eq!(
            to_latex("The first paragraph is\nwrapped here.\n\nThe second follows, with $x$."),
            "The first paragraph is wrapped here.\n\nThe second follows, with $x$."
        );
    }

    #[test]
    fn a_display_is_always_a_display_and_text_after_it_continues_the_paragraph() {
        assert_eq!(
            to_latex("The sum\n\n$$\\sum_{k=1}^n k$$\n\nequals the number."),
            "The sum\n\\[\n\\sum_{k=1}^n k\n\\]\nequals the number."
        );
    }

    #[test]
    fn display_environments_are_not_wrapped_a_second_time() {
        assert_eq!(
            to_latex(
                "We compute\n\n$$ \\begin{align*}a&=1\\\\b&=2\\end{align*} $$\n\nas required."
            ),
            "We compute\n\\begin{align*}a &=1 \\\\ b &=2\\end{align*}\nas required."
        );
    }

    #[test]
    fn specials_are_escaped_and_prices_are_not_math() {
        assert_eq!(
            to_latex("Cost is $5 & 50% off, item #3, file my_name."),
            "Cost is \\$5 \\& 50\\% off, item \\#3, file my\\_name."
        );
        assert_eq!(
            to_latex("It costs $5 now and $10 later, where $x$ is cheap."),
            "It costs \\$5 now and \\$10 later, where $x$ is cheap."
        );
    }

    #[test]
    fn quotes_dashes_and_the_proof_box_are_latex() {
        assert_eq!(
            to_latex("He said, \"Hello—world\" and read pages 10–20. “Done”"),
            "He said, ``Hello---world'' and read pages 10--20. ``Done''"
        );
    }

    #[test]
    fn the_end_of_proof_box_goes_flush_right_whether_alone_or_inline() {
        assert_eq!(
            to_latex("Then $x>0$.\n\n□"),
            "Then $x>0$. \\hfill $\\square$"
        );
        assert_eq!(to_latex("Then $x>0$. □"), "Then $x>0$. \\hfill $\\square$");
    }

    #[test]
    fn markdown_emphasis_becomes_commands_but_spaced_stars_do_not() {
        assert_eq!(
            to_latex("**Problem 3.** Show it, *carefully*."),
            "\\textbf{Problem 3.} Show it, \\emph{carefully}."
        );
        assert_eq!(to_latex("wait * until * done"), "wait * until * done");
    }

    #[test]
    fn lines_of_equal_width_with_a_number_column_are_a_table() {
        let latex = to_latex(
            "Id Name Dept Year Score Grade\n\n1 Ann Math 2021 91 A\n\n2 Bob Art 2022 78 C\n\n3 Cy Bio 2020 85 B",
        );
        assert_eq!(
            latex,
            "\\begin{center}\n\\begin{tabular}{rllrrl}\n\\hline\nId & Name & Dept & Year & Score & Grade \\\\\n\\hline\n1 & Ann & Math & 2021 & 91 & A \\\\\n2 & Bob & Art & 2022 & 78 & C \\\\\n3 & Cy & Bio & 2020 & 85 & B \\\\\n\\hline\n\\end{tabular}\n\\end{center}"
        );
    }

    #[test]
    fn a_two_column_table_needs_distinct_labels() {
        let table = to_latex("First table:\n\nA 1\n\nB 2\n\nC 3");
        assert!(
            table.starts_with("First table:\n\\begin{center}"),
            "{table}"
        );
        assert!(table.contains("{lr}"), "{table}");
        // a repeated label is a list of headings, so no table.
        assert!(!to_latex("Problem 1\n\nProblem 2\n\nProblem 3").contains("tabular"));
    }

    #[test]
    fn ordinary_lines_of_text_are_not_mistaken_for_a_table() {
        // same word count per line but no column of numbers.
        let text = "Show that every tree\n\nProve that every graph\n\nFind the maximum value";
        assert!(!to_latex(text).contains("tabular"));
        // wrapped sentence lines of different lengths.
        let text = "Let G be a connected graph with n vertices and\nm edges. Show that the graph has a spanning tree.";
        assert!(!to_latex(text).contains("tabular"));
    }

    #[test]
    fn code_fences_become_verbatim() {
        assert_eq!(
            to_latex("Run this:\n\n```\nint main() {\n  return 0;\n}\n```\n\nDone."),
            "Run this:\n\\begin{verbatim}\nint main() {\n  return 0;\n}\n\\end{verbatim}\nDone."
        );
    }
}
