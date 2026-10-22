//! cleans up the decoder's raw output, the meaning stays the same.

/// commands whose next {...} group is an argument so the braces stay.
const TAKES_ARG: &[&str] = &[
    "frac",
    "dfrac",
    "tfrac",
    "binom",
    "sqrt",
    "hat",
    "widehat",
    "bar",
    "vec",
    "tilde",
    "widetilde",
    "dot",
    "ddot",
    "overline",
    "underline",
    "overrightarrow",
    "text",
    "textbf",
    "textit",
    "mbox",
    "mathrm",
    "mathbf",
    "mathit",
    "mathcal",
    "mathbb",
    "mathsf",
    "mathtt",
    "boldsymbol",
    "operatorname",
    "begin",
    "end",
    "stackrel",
    "overset",
    "underset",
    "xrightarrow",
];

/// old style font switches and their new commands.
const FONT_SWITCHES: &[(&str, &str)] = &[
    ("\\bf", "\\mathbf"),
    ("\\rm", "\\mathrm"),
    ("\\cal", "\\mathcal"),
    ("\\it", "\\mathit"),
    ("\\sf", "\\mathsf"),
    ("\\tt", "\\mathtt"),
];

/// names that have a built in command.
const OPERATORS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh",
    "coth", "log", "ln", "lg", "exp", "lim", "limsup", "liminf", "max", "min", "sup", "inf", "det",
    "dim", "gcd", "ker", "deg", "arg", "hom", "Pr",
];

/// what kind of thing the model read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// a formula, wrapping is up to the caller.
    Math,
    /// a sentence with its own $...$.
    Prose,
}

#[cfg(test)]
pub fn tidy(raw: &str) -> String {
    tidy_with_kind(raw).0
}

/// tidy the model's output for sentences and formulas.
pub fn tidy_with_kind(raw: &str) -> (String, Kind) {
    let s = compact(raw);
    match crate::prose::convert(&s, &|m| tidy_math(m.to_string())) {
        Some(text) => (text, Kind::Prose),
        None => (tidy_math(s), Kind::Math),
    }
}

/// the formula rules for math the vision engine wrote. It is already normal LaTeX, so it isn't compacted.
pub fn tidy_formula(math: &str) -> String {
    tidy_math(math.to_string())
}

/// the formula rules, on compacted text.
fn tidy_math(s: String) -> String {
    let s = modernize_fonts(&s);
    let s = known_operators(&s);
    let s = delimited_matrices(&s);
    let s = unwrap_matrix_cells(&s);
    let s = binomials(&s);
    let s = mathrm_to_text(&s);
    let s = collapse_double_braces(&s);
    let s = unbrace_scripts(&s);
    // a trailing row break before \end is noise.
    let s = s.replace("\\\\ \\end{", "\\end{");
    let s = drop_wide_spaces(&s);
    trim_spacing(&s)
}

/// `\;` and `\:` between plain symbols are spacing nobody types by hand. Next to a command or a
/// group they can be doing real work, so those stay.
fn drop_wide_spaces(s: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "+-=<>()[]|,.".contains(c);
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\'
            && let Some(&next) = chars.get(i + 1)
        {
            let wide = next == ';' || next == ':';
            if wide
                && out.chars().next_back().is_some_and(plain)
                && !ends_with_control_word(&out)
                && chars.get(i + 2).copied().is_some_and(plain)
            {
                i += 2;
                continue;
            }
            out.push('\\');
            out.push(next);
            i += 2;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// does the text end in a command name like `\alpha`, so a letter after it would join the name.
fn ends_with_control_word(s: &str) -> bool {
    let head = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    head.len() < s.len() && head.chars().rev().take_while(|&c| c == '\\').count() % 2 == 1
}

/// spacing macros that can end up at either end, the model reads an equation number as a run of them.
const SPACING: &[&str] = &["\\qquad", "\\quad", "\\,", "\\;", "\\:", "\\!", "\\ "];

/// drop spacing macros at the very start and end of a formula, they don't separate anything.
fn trim_spacing(s: &str) -> String {
    let mut s = s.trim();
    loop {
        let before = s;
        for m in SPACING {
            // a `\` row break followed by a space or a comma is not spacing.
            if let Some(head) = s.strip_suffix(m)
                && !head.ends_with('\\')
            {
                s = head.trim_end();
            }
            if let Some(tail) = s.strip_prefix(m) {
                s = tail.trim_start();
            }
        }
        if s == before {
            return s.to_string();
        }
    }
}

/// \overline{{G}} to \overline{G}, an outer group with one inner group.
fn collapse_double_braces(s: &str) -> String {
    let mut s = s.to_string();
    let mut i = 0;
    while i + 1 < s.len() {
        let b = s.as_bytes();
        if b[i] == b'{'
            && b[i + 1] == b'{'
            && (i == 0 || b[i - 1] != b'\\')
            && let Some(outer_end) = matching_brace(&s, i)
            && matching_brace(&s, i + 1) == Some(outer_end - 1)
        {
            s.remove(outer_end);
            s.remove(i);
            continue; // the inner group may be doubled too.
        }
        i += 1;
    }
    s
}

/// drop all whitespace except the space TeX needs after a control word.
fn compact(s: &str) -> String {
    let pad = |out: &mut String| {
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    };
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let (mut after_control_word, mut pending_space) = (false, false);

    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && after_control_word && c.is_ascii_alphabetic() {
            out.push(' ');
        }
        pending_space = false;
        after_control_word = false;

        match c {
            '\\' => match chars.peek().copied() {
                Some(n) if n.is_ascii_alphabetic() => {
                    out.push('\\');
                    while let Some(&l) = chars.peek().filter(|l| l.is_ascii_alphabetic()) {
                        out.push(l);
                        chars.next();
                    }
                    after_control_word = true;
                }
                Some('\\') => {
                    chars.next();
                    pad(&mut out);
                    out.push_str("\\\\ ");
                }
                // control symbols like \, \{ and \ .
                Some(n) => {
                    chars.next();
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            },
            '&' => {
                pad(&mut out);
                out.push_str("& ");
            }
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

/// index of the } that matches the { at open.
pub(crate) fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let (mut depth, mut i) = (0usize, open);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1, // skip the escaped character.
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// can a {...} group be replaced by a bare command without changing the grouping.
fn group_is_removable(s: &str, start: usize) -> bool {
    let Some(&prev) = start.checked_sub(1).and_then(|i| s.as_bytes().get(i)) else {
        return true;
    };
    match prev {
        b'{'
        | b'('
        | b'['
        | b','
        | b'='
        | b'+'
        | b'-'
        | b'&'
        | b';'
        | b'|'
        | b'/'
        | b'<'
        | b'>'
        | b' '
        | b'0'..=b'9' => true,
        b'a'..=b'z' | b'A'..=b'Z' => {
            // end of a control word or a plain variable.
            let head = &s[..start];
            let word_start = head
                .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                .len();
            match head[..word_start].strip_suffix('\\') {
                Some(_) => !TAKES_ARG.contains(&&head[word_start..]),
                None => true,
            }
        }
        _ => false, // ^, _, } and so on.
    }
}

/// {\bf E} to \mathbf{E}.
fn modernize_fonts(s: &str) -> String {
    let mut s = s.to_string();
    loop {
        // take the rightmost match so the group can't contain another.
        let found = FONT_SWITCHES
            .iter()
            .filter_map(|&(old, new)| s.rfind(&format!("{{{old} ")).map(|i| (i, old, new)))
            .max_by_key(|&(i, ..)| i);
        let Some((start, old, new)) = found else {
            break;
        };
        let Some(end) = matching_brace(&s, start) else {
            break;
        };
        let body = s[start + old.len() + 2..end].to_string();
        let repl = if group_is_removable(&s, start) {
            format!("{new}{{{body}}}")
        } else {
            format!("{{{new}{{{body}}}}}")
        };
        s.replace_range(start..=end, &repl);
    }
    s
}

/// \operatorname*{lim} to \lim and the like.
fn known_operators(s: &str) -> String {
    const PREFIX: &str = "\\operatorname";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(PREFIX) {
        out.push_str(&rest[..i]);
        let after = &rest[i + PREFIX.len()..];
        let args = after.strip_prefix('*').unwrap_or(after);
        let name = args
            .strip_prefix('{')
            .and_then(|a| a.split_once('}'))
            .filter(|(name, _)| OPERATORS.contains(name));
        match name {
            Some((name, tail)) => {
                out.push('\\');
                out.push_str(name);
                if tail.starts_with(|c: char| c.is_ascii_alphabetic()) {
                    out.push(' ');
                }
                rest = tail;
            }
            None => {
                out.push_str(PREFIX);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// x^{2} to x^2 for single alphanumeric characters.
fn unbrace_scripts(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < c.len() {
        if matches!(c[i], '^' | '_')
            && c.get(i + 1) == Some(&'{')
            && c.get(i + 2).is_some_and(|x| x.is_ascii_alphanumeric())
            && c.get(i + 3) == Some(&'}')
        {
            out.push(c[i]);
            out.push(c[i + 2]);
            i += 4;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// how an environment lays out its cells.
#[derive(Clone, Copy, PartialEq)]
enum Cells {
    /// matrices and cases, cells are joined with ` & `.
    Grid,
    /// array, the first group is the column spec and stays as it is.
    Array,
    /// aligned and friends, a cell that starts with a relation hugs the `&`.
    Align,
}

/// environments where the model wraps cells in extra braces.
const CELL_ENVS: &[(&str, Cells)] = &[
    ("matrix", Cells::Grid),
    ("pmatrix", Cells::Grid),
    ("bmatrix", Cells::Grid),
    ("vmatrix", Cells::Grid),
    ("Vmatrix", Cells::Grid),
    ("Bmatrix", Cells::Grid),
    ("cases", Cells::Grid),
    ("array", Cells::Array),
    ("aligned", Cells::Align),
    ("gathered", Cells::Align),
    ("split", Cells::Align),
    ("align", Cells::Align),
    ("align*", Cells::Align),
];

/// what a cell can start with to be a relation, as in `x &= 1`.
const RELATIONS: &[&str] = &[
    "=",
    "<",
    ">",
    "\\leq",
    "\\le",
    "\\geq",
    "\\ge",
    "\\neq",
    "\\ne",
    "\\approx",
    "\\equiv",
    "\\sim",
    "\\simeq",
    "\\to",
    "\\subset",
    "\\subseteq",
    "\\in",
];

fn starts_with_relation(cell: &str) -> bool {
    RELATIONS.iter().any(|r| {
        cell.strip_prefix(r).is_some_and(|rest| {
            !r.starts_with('\\') || !rest.starts_with(|c: char| c.is_ascii_alphabetic())
        })
    })
}

/// \left<d1>\begin{matrix}..\end{matrix}\right<d2> to the named environment.
const MATRIX_DELIMS: &[(&str, &str, &str)] = &[
    ("(", ")", "pmatrix"),
    ("[", "]", "bmatrix"),
    ("|", "|", "vmatrix"),
    ("\\|", "\\|", "Vmatrix"),
    ("\\{", "\\}", "Bmatrix"),
    ("\\{", ".", "cases"),
];

/// byte index of the \end{env} that closes the environment.
fn find_env_end(s: &str, env: &str) -> Option<usize> {
    let (begin, end) = (format!("\\begin{{{env}}}"), format!("\\end{{{env}}}"));
    let (mut depth, mut i) = (0usize, 0);
    while i < s.len() {
        if s[i..].starts_with(&end) {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
            i += end.len();
        } else if s[i..].starts_with(&begin) {
            depth += 1;
            i += begin.len();
        } else {
            i += s[i..].chars().next()?.len_utf8();
        }
    }
    None
}

/// \left(\begin{matrix}..\end{matrix}\right) to pmatrix and friends.
fn delimited_matrices(s: &str) -> String {
    const OPEN: &str = "\\begin{matrix}";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\left") {
        out.push_str(&rest[..i]);
        let after = &rest[i + "\\left".len()..];
        let converted = MATRIX_DELIMS.iter().find_map(|&(l, r, env)| {
            let body_and_tail = after.strip_prefix(l)?.strip_prefix(OPEN)?;
            let end = find_env_end(body_and_tail, "matrix")?;
            let tail = body_and_tail[end + "\\end{matrix}".len()..]
                .strip_prefix("\\right")?
                .strip_prefix(r)?;
            Some((env, &body_and_tail[..end], tail))
        });
        match converted {
            Some((env, body, tail)) => {
                out.push_str(&format!("\\begin{{{env}}}{body}\\end{{{env}}}"));
                rest = tail;
            }
            None => {
                out.push_str("\\left");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// remove the braces around every cell and a trailing empty row.
fn unwrap_cells(body: &str, style: Cells) -> String {
    let chars: Vec<char> = body.chars().collect();
    let mut rows: Vec<Vec<String>> = vec![vec![String::new()]];
    // braces and nested environments hide their own `&` and `\\`.
    let (mut depth, mut env_depth, mut i) = (0i32, 0i32, 0usize);
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let Some(&n) = chars.get(i + 1) else { break };
            if n == '\\' && depth == 0 && env_depth == 0 {
                rows.push(vec![String::new()]);
                i += 2;
                continue;
            }
            let cell = rows.last_mut().unwrap().last_mut().unwrap();
            cell.push('\\');
            cell.push(n);
            i += 2;
            if n.is_ascii_alphabetic() {
                let mut word = n.to_string();
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    word.push(chars[i]);
                    cell.push(chars[i]);
                    i += 1;
                }
                match word.as_str() {
                    "begin" => env_depth += 1,
                    "end" => env_depth -= 1,
                    _ => {}
                }
            }
            continue;
        }
        if c == '&' && depth == 0 && env_depth == 0 {
            rows.last_mut().unwrap().push(String::new());
        } else {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            rows.last_mut().unwrap().last_mut().unwrap().push(c);
        }
        i += 1;
    }
    if rows.len() > 1
        && rows
            .last()
            .is_some_and(|r| r.len() == 1 && r[0].trim().is_empty())
    {
        rows.pop();
    }
    rows.iter()
        .map(|row| {
            let mut out = String::new();
            for (k, cell) in row.iter().enumerate() {
                let cell = unwrap_cell(cell, style);
                if k > 0 {
                    out.push_str(if style == Cells::Align && starts_with_relation(&cell) {
                        " &"
                    } else {
                        " & "
                    });
                }
                out.push_str(&cell);
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" \\\\ ")
}

/// one cell without the group the model put around it, and in aligned text without the `{}` that
/// stands in for the empty left side of `&=`.
fn unwrap_cell(cell: &str, style: Cells) -> String {
    let cell = cell.trim();
    let mut cell = match matching_brace(cell, 0) {
        Some(end) if cell.starts_with('{') && end == cell.len() - 1 => cell[1..end].trim(),
        _ => cell,
    };
    if style == Cells::Align
        && let Some(rest) = cell.strip_prefix("{}")
        && starts_with_relation(rest.trim_start())
    {
        cell = rest.trim_start();
    }
    cell.to_string()
}

fn unwrap_matrix_cells(s: &str) -> String {
    let mut s = s.to_string();
    for (env, style) in CELL_ENVS {
        let (begin, end_tag) = (format!("\\begin{{{env}}}"), format!("\\end{{{env}}}"));
        let mut out = String::with_capacity(s.len());
        let mut rest = s.as_str();
        while let Some(i) = rest.find(&begin) {
            let body_start = i + begin.len();
            match find_env_end(&rest[body_start..], env) {
                Some(len) => {
                    out.push_str(&rest[..body_start]);
                    let body = &rest[body_start..body_start + len];
                    // an array's first group is its column spec.
                    let (spec, cells) = match matching_brace(body, 0) {
                        Some(end) if *style == Cells::Array && body.starts_with('{') => {
                            body.split_at(end + 1)
                        }
                        _ => ("", body),
                    };
                    out.push_str(spec);
                    out.push_str(&unwrap_cells(cells, *style));
                    rest = &rest[body_start + len..];
                    out.push_str(&end_tag);
                    rest = &rest[end_tag.len()..];
                }
                None => break,
            }
        }
        out.push_str(rest);
        s = out;
    }
    s
}

/// \left(n\atop k\right) to \binom{n}{k}.
fn binomials(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("\\left(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + "\\left(".len()..];
        match binom_parts(after) {
            Some((a, b, tail)) => {
                out.push_str(&format!("\\binom{{{a}}}{{{b}}}"));
                rest = tail;
            }
            None => {
                out.push_str("\\left(");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// text after \left( that looks like A\atop B\right).
fn binom_parts(after: &str) -> Option<(String, String, &str)> {
    let (mut brace, mut nest, mut i) = (0i32, 0i32, 0);
    let mut atop = None;
    while i < after.len() {
        let t = &after[i..];
        if t.starts_with("\\left") {
            nest += 1;
            i += 5;
        } else if let Some(after_right) = t.strip_prefix("\\right") {
            if nest == 0 {
                let tail = after_right.strip_prefix(')')?;
                let inner = &after[..i];
                let at = atop?;
                let (a, b) = (inner[..at].trim(), inner[at + "\\atop".len()..].trim());
                return Some((a.into(), b.into(), tail));
            }
            nest -= 1;
            i += 6;
        } else if t.starts_with("\\atop")
            && !t["\\atop".len()..].starts_with(|c: char| c.is_ascii_alphabetic())
        {
            if brace == 0 && nest == 0 {
                if atop.is_some() {
                    return None; // two \atop at the top level means no binomial.
                }
                atop = Some(i);
            }
            i += 5;
        } else {
            match t.as_bytes()[0] {
                b'\\' => i += 1, // skip the escaped character below.
                b'{' => brace += 1,
                b'}' => brace -= 1,
                _ => {}
            }
            i += after[i..].chars().next()?.len_utf8();
        }
    }
    None
}

/// \mathrm{if~} to \text{if }, only when ~ or \ shows it's real text.
fn mathrm_to_text(s: &str) -> String {
    const OPEN: &str = "\\mathrm{";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(OPEN) {
        out.push_str(&rest[..i]);
        let open = i + OPEN.len() - 1;
        let converted = matching_brace(rest, open).and_then(|end| {
            let inner = &rest[open + 1..end];
            let spaced = inner.contains('~') || inner.contains("\\ ");
            let plain = inner.replace("\\ ", " ").replace('~', " ");
            let plain_ok = plain
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || " .,:;'()-".contains(c));
            (spaced && plain_ok).then_some((plain, end))
        });
        match converted {
            Some((text, end)) => {
                out.push_str(&format!("\\text{{{text}}}"));
                rest = &rest[end + 1..];
            }
            None => {
                out.push_str(OPEN);
                rest = &rest[i + OPEN.len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::{
        drop_wide_spaces, ends_with_control_word, starts_with_relation, tidy, trim_spacing,
    };

    // inputs below are real model output for pdflatex formulas.
    #[test]
    fn compacts_and_unbraces() {
        assert_eq!(tidy("E = m c ^ { 2 }"), "E=mc^2");
        assert_eq!(
            tidy(r"\int _ { 0 } ^ { \infty } e ^ { - x ^ { 2 } } \, d x = \frac { \sqrt \pi } 2"),
            r"\int_0^{\infty}e^{-x^2}\,dx=\frac{\sqrt\pi}2"
        );
    }

    #[test]
    fn keeps_space_between_control_word_and_letter() {
        assert_eq!(tidy(r"\partial t + \alpha x"), r"\partial t+\alpha x");
        assert_eq!(tidy(r"\pi 2"), r"\pi2");
    }

    #[test]
    fn keeps_explicit_spaces() {
        assert_eq!(tidy(r"\mathrm { s u c h \ t h a t }"), r"\text{such that}");
        assert_eq!(tidy(r"\mathrm { i f ~ } x"), r"\text{if }x");
        // no explicit space so it could be an operator, stays \mathrm.
        assert_eq!(tidy(r"\mathrm { V a r } ( X )"), r"\mathrm{Var}(X)");
        assert_eq!(tidy(r"\mathrm { d } x"), r"\mathrm{d}x");
    }

    #[test]
    fn modernizes_fonts_only_where_braces_are_redundant() {
        assert_eq!(
            tidy(r"\nabla \times { \bf E } = - \frac { \partial { \bf B } } { \partial t }"),
            r"\nabla\times\mathbf{E}=-\frac{\partial\mathbf{B}}{\partial t}"
        );
        // script and argument positions keep their braces.
        assert_eq!(tidy(r"x ^ { \bf a }"), r"x^{\mathbf{a}}");
        assert_eq!(tidy(r"\frac { a } { \bf b }"), r"\frac{a}{\mathbf{b}}");
        assert_eq!(tidy(r"\sqrt { \bf b }"), r"\sqrt{\mathbf{b}}");
    }

    #[test]
    fn rewrites_known_operators() {
        assert_eq!(
            tidy(
                r"\operatorname* { l i m } _ { x \to 0 } \frac { \operatorname { s i n } x } { x } = 1"
            ),
            r"\lim_{x\to0}\frac{\sin x}{x}=1"
        );
        assert_eq!(
            tidy(r"\operatorname { f o o } ( x )"),
            r"\operatorname{foo}(x)"
        );
    }

    #[test]
    fn collapses_redundant_double_braces() {
        assert_eq!(tidy(r"\overline { { G } }"), r"\overline{G}");
        assert_eq!(tidy(r"\frac { { a } } { b }"), r"\frac{a}{b}");
        // two groups side by side or with more inner parts are left alone.
        assert_eq!(tidy(r"{ { a } { b } }"), r"{{a}{b}}");
        assert_eq!(tidy(r"x ^ { { \bf y } z }"), r"x^{\mathbf{y}z}");
    }

    #[test]
    fn pads_matrix_separators_and_drops_trailing_break() {
        assert_eq!(
            tidy(r"\begin{matrix} { a } & { b } \\ { c } & { d } \\ \end{matrix}"),
            r"\begin{matrix}a & b \\ c & d\end{matrix}"
        );
    }

    #[test]
    fn names_delimited_matrices() {
        assert_eq!(
            tidy(
                r"A = \left( \begin{matrix} { a } & { b } \\ { c } & { d } \\ \end{matrix} \right)"
            ),
            r"A=\begin{pmatrix}a & b \\ c & d\end{pmatrix}"
        );
        assert_eq!(
            tidy(
                r"\left[ \begin{matrix} { 1 } & { \cdots } & { n } \\ { \vdots } & { \ddots } & { n ^ { 2 } } \end{matrix} \right]"
            ),
            r"\begin{bmatrix}1 & \cdots & n \\ \vdots & \ddots & n^2\end{bmatrix}"
        );
        assert_eq!(
            tidy(r"\left\{ \begin{matrix} { x + y = 2 } \\ { x - y = 0 } \end{matrix} \right."),
            r"\begin{cases}x+y=2 \\ x-y=0\end{cases}"
        );
        // delimiters with no matching environment are left alone.
        assert_eq!(
            tidy(r"\left( \begin{matrix} a \end{matrix} \right]"),
            r"\left(\begin{matrix}a\end{matrix}\right]"
        );
        assert_eq!(tidy(r"\leftarrow x"), r"\leftarrow x");
    }

    #[test]
    fn unwraps_cells_in_cases_and_keeps_text() {
        assert_eq!(
            tidy(
                r"f ( x ) = \begin{cases} { x ^ { 2 } } & { \mathrm { i f ~ } x \geq 0 } \\ { - x } & { \mathrm { o t h e r w i s e } } \\ \end{cases}"
            ),
            r"f(x)=\begin{cases}x^2 & \text{if }x\geq0 \\ -x & \mathrm{otherwise}\end{cases}"
        );
        // two adjacent groups in one cell stay as they are.
        assert_eq!(
            tidy(r"\begin{matrix} { a } { b } & c \end{matrix}"),
            r"\begin{matrix}{a}{b} & c\end{matrix}"
        );
    }

    #[test]
    fn turns_atop_in_delimiters_into_binom() {
        assert_eq!(
            tidy(r"\sum _ { k = 0 } ^ { n } \left( n \atop k \right) x ^ { k }"),
            r"\sum_{k=0}^n\binom{n}{k}x^k"
        );
        // no \atop or a different closing delimiter, no binomial.
        assert_eq!(tidy(r"\left( a + b \right)"), r"\left(a+b\right)");
        assert_eq!(tidy(r"\left( n \atop k \right]"), r"\left(n\atop k\right]");
    }

    #[test]
    fn spacing_macros_at_the_ends_are_dropped() {
        assert_eq!(trim_spacing("x=1\\qquad\\qquad\\qquad"), "x=1");
        assert_eq!(trim_spacing("\\quad x=1 \\, "), "x=1");
        assert_eq!(trim_spacing("a\\;\\:\\!"), "a");
    }

    #[test]
    fn spacing_between_things_and_row_breaks_stay() {
        assert_eq!(trim_spacing("x=1\\qquad y=2"), "x=1\\qquad y=2");
        assert_eq!(trim_spacing("\\int f\\,dx"), "\\int f\\,dx");
        assert_eq!(trim_spacing("a \\\\ "), "a \\\\");
        assert_eq!(trim_spacing("a\\\\,"), "a\\\\,");
    }

    #[test]
    fn an_equation_number_read_as_spacing_leaves_no_trace() {
        let raw = r"\int _ { - \infty } ^ { \infty } e ^ { - x ^ { 2 } } \, d x = \sqrt { \pi } . \qquad \qquad \qquad";
        assert_eq!(
            tidy(raw),
            r"\int_{-\infty}^{\infty}e^{-x^2}\,dx=\sqrt{\pi}."
        );
    }

    #[test]
    fn aligned_cells_lose_their_braces_and_the_empty_group_before_equals() {
        let raw = r"\begin{aligned} { x_1 } & { { } = a_1 + b_1 ^ { 2 } } \\ { x_2 } & { { } = a_2 + b_2 ^ { 2 } } \end{aligned}";
        assert_eq!(
            tidy(raw),
            r"\begin{aligned}x_1 &=a_1+b_1^2 \\ x_2 &=a_2+b_2^2\end{aligned}"
        );
    }

    #[test]
    fn an_array_keeps_its_column_spec_and_loses_the_cell_braces() {
        let raw = r"\begin{array}{ccc} { A } & { \to } & { B } \\ { \downarrow } & { } & { \downarrow } \end{array}";
        assert_eq!(
            tidy(raw),
            r"\begin{array}{ccc}A & \to & B \\ \downarrow &  & \downarrow\end{array}"
        );
    }

    #[test]
    fn a_cell_that_starts_with_a_command_that_only_looks_like_a_relation_gets_a_space() {
        assert!(starts_with_relation("=1"));
        assert!(starts_with_relation(r"\leq 2"));
        assert!(
            !starts_with_relation(r"\leftarrow"),
            "that is a longer command"
        );
        assert!(!starts_with_relation("x=1"));
    }

    #[test]
    fn a_matrix_inside_an_aligned_cell_keeps_its_own_columns() {
        let raw = r"\begin{aligned} { A } & { { } = \begin{pmatrix} { 1 } & { 2 } \\ { 3 } & { 4 } \end{pmatrix} } \end{aligned}";
        assert_eq!(
            tidy(raw),
            r"\begin{aligned}A &=\begin{pmatrix}1 & 2 \\ 3 & 4\end{pmatrix}\end{aligned}"
        );
    }

    #[test]
    fn braces_that_belong_to_the_math_stay() {
        let raw = r"\begin{aligned} x & = \frac { 1 } { 2 } \\ y & = z ^ { n } \end{aligned}";
        assert_eq!(
            tidy(raw),
            r"\begin{aligned}x &=\frac{1}{2} \\ y &=z^n\end{aligned}"
        );
    }

    #[test]
    fn wide_spaces_between_plain_symbols_are_dropped() {
        assert_eq!(drop_wide_spaces(r"a\;b"), "ab");
        assert_eq!(drop_wide_spaces(r"x=1\;,\;y"), "x=1,y");
        assert_eq!(drop_wide_spaces(r"f(x)\:=\:2"), "f(x)=2");
    }

    #[test]
    fn wide_spaces_next_to_commands_and_groups_stay() {
        for keep in [
            r"\alpha\;b",
            r"x\;\leq\;y",
            r"x>0\;\text{and}\;y>0",
            r"\mathrm{d}\;x",
            r"a\\;b",
        ] {
            assert_eq!(drop_wide_spaces(keep), keep, "{keep}");
        }
    }

    #[test]
    fn a_command_name_is_recognised_at_the_end_of_text() {
        assert!(ends_with_control_word(r"\alpha"));
        assert!(ends_with_control_word(r"x+\int"));
        assert!(!ends_with_control_word(r"x"));
        assert!(
            !ends_with_control_word(r"a\\b"),
            "a row break then a letter"
        );
    }
}
