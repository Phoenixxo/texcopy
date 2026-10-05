//! Post-processing of the decoder's raw output.
//!
//! The model was trained on LaTeX normalised to one token per symbol, so its output
//! looks like `\frac { a } { b }` or `\mathrm { s i n }`. Every transformation here
//! keeps the formula's meaning and only makes it read like hand-written LaTeX.
//! Real spaces never get lost: the model spells them out as `~` or `\ `.

/// Commands whose next `{...}` group is an argument, so its braces must stay.
const TAKES_ARG: &[&str] = &[
    "frac", "dfrac", "tfrac", "binom", "sqrt", "hat", "widehat", "bar", "vec", "tilde", "widetilde",
    "dot", "ddot", "overline", "underline", "overrightarrow", "text", "textbf", "textit", "mbox",
    "mathrm", "mathbf", "mathit", "mathcal", "mathbb", "mathsf", "mathtt", "boldsymbol",
    "operatorname", "begin", "end", "stackrel", "overset", "underset", "xrightarrow",
];

/// Old-style font switches and their modern command equivalents.
const FONT_SWITCHES: &[(&str, &str)] = &[
    ("\\bf", "\\mathbf"),
    ("\\rm", "\\mathrm"),
    ("\\cal", "\\mathcal"),
    ("\\it", "\\mathit"),
    ("\\sf", "\\mathsf"),
    ("\\tt", "\\mathtt"),
];

/// Names that have a built-in command (`\sin`, `\lim`, ...).
const OPERATORS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh",
    "coth", "log", "ln", "lg", "exp", "lim", "limsup", "liminf", "max", "min", "sup", "inf", "det",
    "dim", "gcd", "ker", "deg", "arg", "hom", "Pr",
];

pub fn tidy(raw: &str) -> String {
    let s = compact(raw);
    let s = modernize_fonts(&s);
    let s = known_operators(&s);
    let s = unbrace_scripts(&s);
    // A trailing row break before \end{...} is noise the model likes to add.
    s.replace("\\\\ \\end{", "\\end{")
}

/// Drop all whitespace except the single space TeX needs between a control word
/// and a following letter. Row separators (`&`, `\\`) get readable padding.
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
                // Control symbols such as `\,` `\{` and `\ ` (the latter is a real space).
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

/// Index of the `}` matching the `{` at `open`.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let b = s.as_bytes();
    let (mut depth, mut i) = (0usize, open);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1, // skip the escaped character
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

/// Can a `{...}` group starting at `start` be replaced by a bare command without
/// changing how TeX groups things? Not if it is a script or a command argument.
fn group_is_removable(s: &str, start: usize) -> bool {
    let Some(&prev) = start.checked_sub(1).and_then(|i| s.as_bytes().get(i)) else {
        return true;
    };
    match prev {
        b'{' | b'(' | b'[' | b',' | b'=' | b'+' | b'-' | b'&' | b';' | b'|' | b'/' | b'<' | b'>'
        | b' ' | b'0'..=b'9' => true,
        b'a'..=b'z' | b'A'..=b'Z' => {
            // Either the end of a control word or a plain variable.
            let head = &s[..start];
            let word_start = head.trim_end_matches(|c: char| c.is_ascii_alphabetic()).len();
            match head[..word_start].strip_suffix('\\') {
                Some(_) => !TAKES_ARG.contains(&&head[word_start..]),
                None => true,
            }
        }
        _ => false, // `^`, `_`, `}` ...
    }
}

/// `{\bf E}` -> `\mathbf{E}`, keeping the outer braces where they matter.
fn modernize_fonts(s: &str) -> String {
    let mut s = s.to_string();
    loop {
        // Take the rightmost match so the group cannot contain another one.
        let found = FONT_SWITCHES
            .iter()
            .filter_map(|&(old, new)| s.rfind(&format!("{{{old} ")).map(|i| (i, old, new)))
            .max_by_key(|&(i, ..)| i);
        let Some((start, old, new)) = found else { break };
        let Some(end) = matching_brace(&s, start) else { break };
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

/// `\operatorname*{lim}` -> `\lim`, `\operatorname{sin}` -> `\sin`.
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

/// `x^{2}` -> `x^2`, `a_{i}` -> `a_i` (only for single alphanumeric characters).
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

#[cfg(test)]
mod tests {
    use super::tidy;

    // Inputs below are real model outputs for pdflatex-rendered formulas.
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
        assert_eq!(
            tidy(r"\mathrm { s u c h \ t h a t }"),
            r"\mathrm{such\ that}"
        );
        assert_eq!(tidy(r"\mathrm { i f ~ } x"), r"\mathrm{if~}x");
    }

    #[test]
    fn modernizes_fonts_only_where_braces_are_redundant() {
        assert_eq!(
            tidy(r"\nabla \times { \bf E } = - \frac { \partial { \bf B } } { \partial t }"),
            r"\nabla\times\mathbf{E}=-\frac{\partial\mathbf{B}}{\partial t}"
        );
        // Script and argument positions keep their braces.
        assert_eq!(tidy(r"x ^ { \bf a }"), r"x^{\mathbf{a}}");
        assert_eq!(tidy(r"\frac { a } { \bf b }"), r"\frac{a}{\mathbf{b}}");
        assert_eq!(tidy(r"\sqrt { \bf b }"), r"\sqrt{\mathbf{b}}");
    }

    #[test]
    fn rewrites_known_operators() {
        assert_eq!(
            tidy(r"\operatorname* { l i m } _ { x \to 0 } \frac { \operatorname { s i n } x } { x } = 1"),
            r"\lim_{x\to0}\frac{\sin x}{x}=1"
        );
        assert_eq!(tidy(r"\operatorname { f o o } ( x )"), r"\operatorname{foo}(x)");
    }

    #[test]
    fn pads_matrix_separators_and_drops_trailing_break() {
        assert_eq!(
            tidy(r"\begin{matrix} { a } & { b } \\ { c } & { d } \\ \end{matrix}"),
            r"\begin{matrix}{a} & {b} \\ {c} & {d} \end{matrix}"
        );
    }
}
