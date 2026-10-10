//! Random formulas from a small grammar. Every method returns a string of LaTeX.
//!
//! The formulas are mathematical nonsense on purpose: what is being tested is whether texcopy reads
//! the symbols, not whether it knows the mathematics.

use crate::rng::Rng;

const GREEK: [&str; 10] = [
    r"\alpha", r"\beta", r"\gamma", r"\delta", r"\theta", r"\lambda", r"\mu", r"\sigma", r"\phi",
    r"\omega",
];
const GREEK_UP: [&str; 4] = [r"\Gamma", r"\Delta", r"\Sigma", r"\Omega"];
const LATIN: [&str; 40] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "k", "m", "n", "p", "q", "r", "s", "t", "u", "v", "w",
    "x", "y", "z", "A", "B", "C", "D", "E", "F", "G", "H", "K", "M", "N", "P", "R", "S", "T", "X",
    "Y", "Z",
];
const SUBS: [&str; 10] = ["i", "j", "k", "n", "m", "0", "1", "2", "ij", "t"];
const FUNCS: [&str; 6] = ["sin", "cos", "tan", "log", "ln", "exp"];
const RELS: [&str; 8] = [
    "=", r"\leq", r"\geq", "<", ">", r"\neq", r"\approx", r"\equiv",
];
const SET_RELS: [&str; 4] = [r"\in", r"\subset", r"\subseteq", r"\to"];
const SETS: [&str; 5] = [
    r"\mathbb{R}",
    r"\mathbb{N}",
    r"\mathbb{Z}",
    r"\mathbb{Q}",
    r"\mathbb{C}",
];
const OPS: [&str; 5] = ["+", "-", r"\cdot", r"\times", r"\pm"];
const CONSTS: [&str; 7] = [r"\pi", r"\infty", "e", "i", "2", "3", "10"];
const ACCENTS: [&str; 5] = ["hat", "bar", "vec", "tilde", "overline"];

pub struct FormulaGen<'a> {
    pub r: &'a mut Rng,
}

impl<'a> FormulaGen<'a> {
    pub fn new(r: &'a mut Rng) -> Self {
        FormulaGen { r }
    }

    fn latin(&mut self) -> &'static str {
        self.r.pick(&LATIN)
    }

    /// A letter, Greek letter or capital Greek letter.
    fn letter(&mut self) -> &'static str {
        if self.r.chance(0.75) {
            let i = self
                .r
                .int(0, (LATIN.len() + GREEK.len() + GREEK_UP.len()) as i64 - 1)
                as usize;
            if i < LATIN.len() {
                LATIN[i]
            } else if i < LATIN.len() + GREEK.len() {
                GREEK[i - LATIN.len()]
            } else {
                GREEK_UP[i - LATIN.len() - GREEK.len()]
            }
        } else {
            self.latin()
        }
    }

    pub fn var(&mut self) -> String {
        let mut out = self.letter().to_string();
        if self.r.chance(0.35) {
            let sub = *self.r.pick(&SUBS);
            out += &if sub.len() > 1 {
                format!("_{{{sub}}}")
            } else {
                format!("_{sub}")
            };
        }
        if self.r.chance(0.15) {
            // no `{*}` here: with Times math the TeX install used for the benchmark pulls in a missing script font
            out += &format!("^{}", self.r.pick(&["2", "3", "n", "k"]));
        }
        out
    }

    fn num(&mut self) -> String {
        if self.r.chance(0.2) {
            format!("{}.{}", self.r.int(1, 9), self.r.int(1, 99))
        } else {
            self.r.int(0, 99).to_string()
        }
    }

    pub fn atom(&mut self) -> String {
        let x = self.r.float();
        if x < 0.55 {
            self.var()
        } else if x < 0.85 {
            self.num()
        } else {
            (*self.r.pick(&CONSTS)).to_string()
        }
    }

    /// A variable or number with no script of its own, so `x^2^3` cannot happen.
    fn atom_no_script(&mut self) -> String {
        if self.r.chance(0.7) {
            let i = self.r.int(0, (LATIN.len() + GREEK.len()) as i64 - 1) as usize;
            (if i < LATIN.len() {
                LATIN[i]
            } else {
                GREEK[i - LATIN.len()]
            })
            .to_string()
        } else {
            self.num()
        }
    }

    pub fn expr(&mut self, depth: i64) -> String {
        if depth <= 0 {
            return self.atom();
        }
        let d = depth - 1;
        let kind = *self.r.weighted(&[
            ("op", 22),
            ("frac", 14),
            ("pow", 10),
            ("sqrt", 7),
            ("func", 8),
            ("paren", 8),
            ("abs", 4),
            ("bigop", 10),
            ("binom", 3),
            ("accent", 5),
            ("atom", 9),
        ]);
        match kind {
            "op" => format!("{} {} {}", self.expr(d), self.r.pick(&OPS), self.expr(d)),
            "frac" => format!(r"\frac{{{}}}{{{}}}", self.expr(d), self.expr(d)),
            "pow" => {
                let exponent = if self.r.chance(0.6) {
                    self.expr(0)
                } else {
                    self.expr(d)
                };
                format!("{}^{{{}}}", self.atom_no_script(), exponent)
            }
            "sqrt" => {
                let root = if self.r.chance(0.15) {
                    format!(r"\sqrt[{}]", self.r.pick(&["3", "n"]))
                } else {
                    r"\sqrt".to_string()
                };
                format!("{root}{{{}}}", self.expr(d))
            }
            "func" => {
                let name = *self.r.pick(&FUNCS);
                if self.r.chance(0.6) {
                    format!(r"\{name}({})", self.expr(d))
                } else {
                    format!(r"\{name} {}", self.atom_no_script())
                }
            }
            "paren" => {
                if self.r.chance(0.5) {
                    format!(r"\left({}\right)", self.expr(d))
                } else {
                    format!("({})", self.expr(d))
                }
            }
            "abs" => {
                if self.r.chance(0.7) {
                    format!("|{}|", self.expr(d))
                } else {
                    format!(r"\left\|{}\right\|", self.expr(d))
                }
            }
            "bigop" => self.bigop(d),
            "binom" => format!(
                r"\binom{{{}}}{{{}}}",
                self.r.pick(&["n", "m", "k+1"]),
                self.r.pick(&["k", "j", "2"])
            ),
            "accent" => format!(r"\{}{{{}}}", self.r.pick(&ACCENTS), self.latin()),
            _ => self.atom(),
        }
    }

    fn bigop(&mut self, d: i64) -> String {
        let op = *self
            .r
            .weighted(&[("sum", 45), ("prod", 15), ("int", 28), ("lim", 12)]);
        let idx = *self.r.pick(&["i", "k", "j", "n"]);
        match op {
            "sum" | "prod" => {
                let lo = format!("{idx}={}", self.r.pick(&["0", "1"]));
                let hi = *self.r.pick(&["n", "m", r"\infty", "N"]);
                format!(r"\{op}_{{{lo}}}^{{{hi}}} {}", self.expr(d.min(1)))
            }
            "int" => {
                let (lo, hi) = *self.r.pick(&[
                    ("0", "1"),
                    ("a", "b"),
                    (r"-\infty", r"\infty"),
                    ("0", r"\pi"),
                ]);
                let v = *self.r.pick(&["x", "t"]);
                format!(r"\int_{{{lo}}}^{{{hi}}} {}\,d{v}", self.expr(d.min(1)))
            }
            _ => format!(
                r"\lim_{{{}\to {}}} {}",
                self.r.pick(&["x", "n"]),
                self.r.pick(&["0", r"\infty", "a"]),
                self.expr(d.min(1))
            ),
        }
    }

    fn relation(&mut self, depth: i64) -> String {
        let lo = self.r.int(0, depth);
        let lhs = self.expr(lo);
        let rhs = self.expr(depth);
        if self.r.chance(0.12) {
            return format!(
                "{} {} {}",
                self.var(),
                self.r.pick(&SET_RELS),
                self.r.pick(&SETS)
            );
        }
        let mut out = format!("{lhs} {} {rhs}", self.r.pick(&RELS));
        if depth >= 2 && self.r.chance(0.2) {
            out += &format!(" {} {}", self.r.pick(&["=", r"\leq"]), self.expr(depth - 1));
        }
        out
    }

    fn cell(&mut self) -> String {
        if self.r.chance(0.75) {
            self.expr(0)
        } else {
            self.expr(1)
        }
    }

    fn matrix(&mut self) -> String {
        let env = *self.r.pick(&["pmatrix", "bmatrix", "vmatrix"]);
        let (rows, cols) = (self.r.int(2, 3), self.r.int(2, 3));
        let body: Vec<String> = (0..rows)
            .map(|_| {
                (0..cols)
                    .map(|_| self.cell())
                    .collect::<Vec<_>>()
                    .join(" & ")
            })
            .collect();
        let lead = if self.r.chance(0.6) {
            format!("{} = ", self.r.pick(&["A", "M", "B", "T"]))
        } else {
            String::new()
        };
        format!(
            r"{lead}\begin{{{env}}} {} \end{{{env}}}",
            body.join(r" \\ ")
        )
    }

    fn cases(&mut self) -> String {
        let n = self.r.int(2, 3) as usize;
        let mut conds = vec![
            r"x \geq 0",
            "x < 0",
            "x = 1",
            r"n \text{ even}",
            r"n \text{ odd}",
            "x > 2",
        ];
        self.r.shuffle(&mut conds);
        let rows: Vec<String> = (0..n)
            .map(|i| format!("{} & {}", self.expr(1), conds[i]))
            .collect();
        format!(
            r"{}(x) = \begin{{cases}} {} \end{{cases}}",
            self.r.pick(&["f", "g", "h"]),
            rows.join(r" \\ ")
        )
    }

    fn aligned(&mut self) -> String {
        let n = self.r.int(2, 3);
        let rows: Vec<String> = (0..n)
            .map(|_| format!("{} &= {}", self.expr(1), self.expr(2)))
            .collect();
        format!(r"\begin{{aligned}} {} \end{{aligned}}", rows.join(r" \\ "))
    }

    /// One formula of the given category (`formula-simple`, `matrix`, ...).
    pub fn formula(&mut self, category: &str) -> String {
        match category {
            "formula-simple" => self.relation(1),
            "formula-medium" => self.relation(2),
            "formula-hard" => {
                if self.r.chance(0.6) {
                    self.relation(3)
                } else {
                    format!("{} = {}", self.bigop(2), self.expr(2))
                }
            }
            "matrix" => self.matrix(),
            "cases" => self.cases(),
            "aligned" => self.aligned(),
            other => panic!("unknown formula category {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_makes_the_same_formula() {
        let make = |seed: &str| FormulaGen::new(&mut Rng::from_text(seed)).formula("formula-hard");
        assert_eq!(make("a"), make("a"));
        assert_ne!(make("a"), make("b"));
    }

    #[test]
    fn formulas_are_balanced_and_never_double_a_script() {
        for i in 0..400 {
            let mut rng = Rng::from_text(&format!("t{i}"));
            let cat = [
                "formula-simple",
                "formula-medium",
                "formula-hard",
                "matrix",
                "cases",
                "aligned",
            ][i % 6];
            let f = FormulaGen::new(&mut rng).formula(cat);
            assert_eq!(
                f.matches('{').count(),
                f.matches('}').count(),
                "unbalanced braces: {f}"
            );
            assert_eq!(
                f.matches(r"\left").count(),
                f.matches(r"\right").count(),
                "unbalanced \\left/\\right: {f}"
            );
            assert!(
                !f.contains("^^") && !f.contains("__"),
                "doubled script: {f}"
            );
            assert!(f.len() < 700, "too long: {f}");
        }
    }
}
