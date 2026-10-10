//! Sentences and small documents: paragraphs, display equations, lists, tables and headings.
//! All the wording is original, written for this generator.

use crate::{formula::FormulaGen, rng::Rng};

const NOUNS: [&str; 10] = [
    "graph",
    "matrix",
    "group",
    "ring",
    "sequence",
    "function",
    "polynomial",
    "vector space",
    "tree",
    "set",
];
const ADJ: [&str; 9] = [
    "connected",
    "finite",
    "bounded",
    "continuous",
    "invertible",
    "nonempty",
    "compact",
    "symmetric",
    "regular",
];
const VERBS: [&str; 4] = ["Show", "Prove", "Verify", "Explain why"];
const TOPICS: [&str; 8] = [
    "Convergence",
    "Counting",
    "Linear Maps",
    "Basic Notation",
    "A Small Example",
    "Estimates",
    "Graph Colouring",
    "Bounds",
];
const HEADERS: [&str; 9] = [
    "Name", "Value", "Count", "Size", "Rank", "Score", "Order", "Type", "Weight",
];
const CELL_WORDS: [&str; 10] = [
    "alpha", "beta", "gamma", "delta", "tree", "cycle", "path", "star", "ring", "chain",
];
const LEAD_INS: [&str; 5] = [
    "We use the following fact:",
    "This gives",
    "Combining these,",
    "In particular,",
    "Hence",
];

pub struct DocGen<'a> {
    pub r: &'a mut Rng,
}

impl<'a> DocGen<'a> {
    pub fn new(r: &'a mut Rng) -> Self {
        DocGen { r }
    }

    fn formulas(&mut self) -> FormulaGen<'_> {
        FormulaGen::new(self.r)
    }

    /// Inline math in dollar signs: a small relation or expression.
    fn inline(&mut self) -> String {
        let simple = self.r.chance(0.6);
        let mut f = self.formulas();
        format!(
            "${}$",
            if simple {
                f.formula("formula-simple")
            } else {
                f.expr(1)
            }
        )
    }

    pub fn sentence(&mut self) -> String {
        let noun = *self.r.pick(&NOUNS);
        let adj = *self.r.pick(&ADJ);
        let verb = *self.r.pick(&VERBS);
        match self.r.int(0, 7) {
            0 => format!(
                "Let {} be a {adj} {noun} and suppose {}.",
                self.inline(),
                self.inline()
            ),
            1 => format!("{verb} that {} whenever {}.", self.inline(), self.inline()),
            2 => format!(
                "If {}, then {} for every {noun}.",
                self.inline(),
                self.inline()
            ),
            3 => format!(
                "Find all values of {} such that {}.",
                self.inline(),
                self.inline()
            ),
            4 => format!(
                "The {noun} {} is {adj} when {}.",
                self.inline(),
                self.inline()
            ),
            5 => format!(
                "We write {} for the {adj} {noun} with {}.",
                self.inline(),
                self.inline()
            ),
            6 => format!(
                "Since {}, it follows that {}.",
                self.inline(),
                self.inline()
            ),
            _ => format!(
                "Consider a {adj} {noun} with {} and {}.",
                self.inline(),
                self.inline()
            ),
        }
    }

    pub fn paragraph(&mut self) -> String {
        let n = self.r.int(1, 3);
        (0..n)
            .map(|_| self.sentence())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn display(&mut self) -> String {
        let category = *self.r.pick(&[
            "formula-medium",
            "formula-medium",
            "formula-hard",
            "aligned",
            "cases",
        ]);
        let body = self.formulas().formula(category);
        if !body.contains("begin{aligned}") && self.r.chance(0.5) {
            format!("\\begin{{equation}}\n{body}\n\\end{{equation}}")
        } else {
            format!("\\[\n{body}\n\\]")
        }
    }

    fn list_block(&mut self) -> String {
        let env = *self.r.pick(&["itemize", "enumerate"]);
        let n = self.r.int(2, 4);
        let items: Vec<String> = (0..n)
            .map(|_| format!("\\item {}", self.sentence()))
            .collect();
        format!("\\begin{{{env}}}\n{}\n\\end{{{env}}}", items.join("\n"))
    }

    fn table_block(&mut self) -> String {
        let cols = self.r.int(2, 4) as usize;
        let rows = self.r.int(2, 4);
        let mut names: Vec<&str> = HEADERS.to_vec();
        self.r.shuffle(&mut names);
        let header = &names[..cols];
        let numeric: Vec<bool> = (0..cols).map(|_| self.r.chance(0.6)).collect();
        let mut lines = Vec::new();
        for _ in 0..rows {
            let cells: Vec<String> = (0..cols)
                .map(|j| {
                    if numeric[j] {
                        self.r.int(0, 99).to_string()
                    } else {
                        (*self.r.pick(&CELL_WORDS)).to_string()
                    }
                })
                .collect();
            lines.push(cells.join(" & "));
        }
        let spec: String = numeric.iter().map(|&n| if n { 'r' } else { 'l' }).collect();
        format!(
            "\\begin{{tabular}}{{{spec}}}\n\\hline\n{} \\\\\n\\hline\n{} \\\\\n\\hline\n\\end{{tabular}}",
            header.join(" & "),
            lines.join(" \\\\\n")
        )
    }

    fn heading(&mut self) -> String {
        format!("\\section*{{{}}}", self.r.pick(&TOPICS))
    }

    /// A few blocks: mostly paragraphs, sometimes a display equation, list, table or heading.
    pub fn document(&mut self) -> String {
        let mut blocks = Vec::new();
        if self.r.chance(0.25) {
            blocks.push(self.heading());
        }
        blocks.push(self.paragraph());
        for _ in 0..self.r.int(1, 2) {
            let kind = *self.r.weighted(&[
                ("display", 38),
                ("list", 22),
                ("table", 18),
                ("paragraph", 22),
            ]);
            blocks.push(match kind {
                "display" => format!("{}\n{}", self.r.pick(&LEAD_INS), self.display()),
                "list" => self.list_block(),
                "table" => self.table_block(),
                _ => self.paragraph(),
            });
        }
        blocks.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_are_deterministic_and_have_balanced_environments() {
        let make = |s: &str| DocGen::new(&mut Rng::from_text(s)).document();
        assert_eq!(make("x"), make("x"));
        for i in 0..200 {
            let d = make(&format!("doc{i}"));
            assert_eq!(
                d.matches("\\begin{").count(),
                d.matches("\\end{").count(),
                "{d}"
            );
            assert_eq!(d.matches('$').count() % 2, 0, "odd number of dollars: {d}");
        }
    }

    #[test]
    fn sentences_have_inline_math_and_end_with_a_full_stop() {
        for i in 0..100 {
            let s = DocGen::new(&mut Rng::from_text(&format!("s{i}"))).sentence();
            assert!(s.contains('$') && s.ends_with('.'), "{s}");
        }
    }
}
