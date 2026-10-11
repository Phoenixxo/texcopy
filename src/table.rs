//! tables, turns the model's HTML table into a LaTeX tabular.

/// a cell as the HTML described it.
struct Cell {
    text: String,
    colspan: usize,
    rowspan: usize,
    header: bool,
}

/// a cell placed on the grid.
struct Placed {
    latex: String,
    /// the cell's plain text, to tell if the column is numbers.
    raw: String,
    colspan: usize,
}

/// where the first <table>...</table> is, end exclusive.
pub fn find(s: &str) -> Option<(usize, usize)> {
    let lower = s.to_ascii_lowercase();
    let start = lower.find("<table")?;
    let end = lower[start..]
        .find("</table>")
        .map_or(s.len(), |e| start + e + "</table>".len());
    Some((start, end))
}

fn attr(tag: &str, name: &str) -> Option<usize> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))? + name.len() + 1;
    let digits: String = lower[at..]
        .trim_start_matches(['"', '\''])
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().filter(|&n| n >= 1)
}

fn decode(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// rows of cells, forgiving about missing closing tags and unknown tags.
fn parse(html: &str) -> Vec<Vec<Cell>> {
    let (mut rows, mut row): (Vec<Vec<Cell>>, Vec<Cell>) = (Vec::new(), Vec::new());
    let mut cell: Option<Cell> = None;
    let mut rest = html;
    let close_cell = |cell: &mut Option<Cell>, row: &mut Vec<Cell>| {
        if let Some(mut c) = cell.take() {
            c.text = decode(&c.text)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            row.push(c);
        }
    };
    while let Some(lt) = rest.find('<') {
        if let Some(c) = cell.as_mut() {
            c.text.push_str(&rest[..lt]);
        }
        let Some(gt) = rest[lt..].find('>') else {
            break;
        };
        let tag = &rest[lt + 1..lt + gt];
        rest = &rest[lt + gt + 1..];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let closing = tag.starts_with('/');
        match (name.as_str(), closing) {
            ("tr", false) => {
                close_cell(&mut cell, &mut row);
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                }
            }
            ("tr", true) => {
                close_cell(&mut cell, &mut row);
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                }
            }
            ("td" | "th", false) => {
                close_cell(&mut cell, &mut row);
                cell = Some(Cell {
                    text: String::new(),
                    colspan: attr(tag, "colspan").unwrap_or(1).min(32),
                    rowspan: attr(tag, "rowspan").unwrap_or(1).min(32),
                    header: name == "th",
                });
            }
            ("td" | "th", true) => close_cell(&mut cell, &mut row),
            ("br", _) => {
                if let Some(c) = cell.as_mut() {
                    c.text.push(' ');
                }
            }
            _ => {}
        }
    }
    close_cell(&mut cell, &mut row);
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// a tabular in center for the HTML table, None if there are no cells.
pub fn to_latex(html: &str, clean: &dyn Fn(&str, bool) -> String) -> Option<String> {
    layout(parse(html), clean)
}

/// a tabular for rows of plain cell text.
pub fn from_rows(rows: &[Vec<String>], clean: &dyn Fn(&str, bool) -> String) -> Option<String> {
    let cells = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|t| Cell {
                    text: t.clone(),
                    colspan: 1,
                    rowspan: 1,
                    header: false,
                })
                .collect()
        })
        .collect();
    layout(cells, clean)
}

/// does text read as a number.
pub fn is_number_like(text: &str) -> bool {
    let t = text
        .trim()
        .trim_start_matches(['+', '-', '\u{2212}', '$', '€', '£', '¥']);
    let t = t.trim_end_matches('%');
    t.chars().any(|c| c.is_ascii_digit())
        && t.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
}

fn layout(rows: Vec<Vec<Cell>>, clean: &dyn Fn(&str, bool) -> String) -> Option<String> {
    if rows.is_empty() {
        return None;
    }
    // place every cell on a grid so colspan and rowspan keep columns lined up.
    let mut carry: Vec<usize> = Vec::new(); // rows still covered by a cell from above, per column.
    let mut grid: Vec<Vec<Option<Placed>>> = Vec::new(); // None means covered by a rowspan from above.
    for row in &rows {
        let (mut out, mut col) = (Vec::new(), 0usize);
        let skip_covered =
            |col: &mut usize, out: &mut Vec<Option<Placed>>, carry: &mut Vec<usize>| {
                while carry.get(*col).is_some_and(|&n| n > 0) {
                    carry[*col] -= 1;
                    out.push(None);
                    *col += 1;
                }
            };
        for cell in row {
            skip_covered(&mut col, &mut out, &mut carry);
            if carry.len() < col + cell.colspan {
                carry.resize(col + cell.colspan, 0);
            }
            carry[col..col + cell.colspan].fill(cell.rowspan - 1);
            out.push(Some(Placed {
                latex: String::new(), // filled in below once we know the header row.
                raw: cell.text.clone(),
                colspan: cell.colspan,
            }));
            col += cell.colspan;
        }
        skip_covered(&mut col, &mut out, &mut carry);
        grid.push(out);
    }
    let row_width =
        |r: &Vec<Option<Placed>>| r.iter().map(|c| c.as_ref().map_or(1, |c| c.colspan)).sum();
    let n_cols: usize = grid.iter().map(row_width).max()?;

    let has_header = rows[0].iter().any(|c| c.header)
        || (rows.len() > 1
            && rows[0]
                .iter()
                .filter(|c| !c.text.is_empty())
                .all(|c| c.text.chars().any(char::is_alphabetic)));

    for (r, row) in grid.iter_mut().enumerate() {
        for placed in row.iter_mut().flatten() {
            placed.latex = clean(&placed.raw, has_header && r == 0);
        }
    }

    // number columns right aligned, text columns left.
    let mut spec = String::new();
    for target in 0..n_cols {
        let (mut numeric, mut seen) = (true, false);
        for row in grid.iter().skip(usize::from(has_header)) {
            let mut col = 0;
            for cell in row {
                let Some(p) = cell else {
                    col += 1;
                    continue;
                };
                if col == target && p.colspan == 1 && !p.raw.is_empty() {
                    seen = true;
                    numeric &= is_number_like(&p.raw);
                }
                col += p.colspan;
            }
        }
        spec.push(if numeric && seen { 'r' } else { 'l' });
    }

    let mut out = format!("\\begin{{center}}\n\\begin{{tabular}}{{{spec}}}\n\\hline\n");
    for (i, row) in grid.iter().enumerate() {
        let cells: Vec<String> = row
            .iter()
            .map(|c| match c {
                None => String::new(),
                Some(p) if p.colspan == 1 => p.latex.clone(),
                Some(p) => format!("\\multicolumn{{{}}}{{c}}{{{}}}", p.colspan, p.latex),
            })
            .collect();
        out.push_str(&format!("{} \\\\\n", cells.join(" & ")));
        if i == 0 && has_header {
            out.push_str("\\hline\n");
        }
    }
    out.push_str("\\hline\n\\end{tabular}\n\\end{center}");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str, _header: bool) -> String {
        s.replace('&', "\\&")
            .replace('%', "\\%")
            .replace('_', "\\_")
    }

    /// what the vision model returned for a small execution time table.
    const TIMES: &str = "<table border=1><tr><td>n</td><td>Recursive time (ms)</td><td>DP time (ms)</td></tr><tr><td>5</td><td>0.000037</td><td>0.000022</td></tr><tr><td>7</td><td>0.000164</td><td>0.000025</td></tr><tr><td>23</td><td>13.314562</td><td>0.000154</td></tr></table>";

    #[test]
    fn a_header_row_number_columns_right_and_rules() {
        let latex = to_latex(TIMES, &|c, h| {
            if c == "n" { "$n$".into() } else { plain(c, h) }
        })
        .unwrap();
        assert_eq!(
            latex,
            "\\begin{center}\n\\begin{tabular}{rrr}\n\\hline\n$n$ & Recursive time (ms) & DP time (ms) \\\\\n\\hline\n5 & 0.000037 & 0.000022 \\\\\n7 & 0.000164 & 0.000025 \\\\\n23 & 13.314562 & 0.000154 \\\\\n\\hline\n\\end{tabular}\n\\end{center}"
        );
    }

    #[test]
    fn finds_the_table_inside_other_text() {
        let s = format!("Execution-time comparison\n\n{TIMES}\n\nDone.");
        let (a, b) = find(&s).unwrap();
        assert!(s[..a].contains("comparison") && s[b..].contains("Done"));
        assert!(s[a..b].ends_with("</table>"));
        assert!(find("no table here").is_none());
    }

    #[test]
    fn th_cells_mark_the_header_and_entities_and_specials_are_handled() {
        let html = "<table><tr><th>Name</th><th>Rate &amp; share</th></tr><tr><td>a_b</td><td>50%</td></tr></table>";
        let latex = to_latex(html, &plain).unwrap();
        assert!(
            latex.contains("Name & Rate \\& share \\\\\n\\hline\n"),
            "{latex}"
        );
        assert!(latex.contains("a\\_b & 50\\% \\\\"), "{latex}");
        assert!(latex.contains("{lr}"), "{latex}");
    }

    #[test]
    fn colspan_becomes_multicolumn_and_rowspan_keeps_the_columns_aligned() {
        let html = "<table><tr><th colspan=2>Both</th><th>C</th></tr><tr><td rowspan=2>x</td><td>1</td><td>2</td></tr><tr><td>3</td><td>4</td></tr></table>";
        let latex = to_latex(html, &plain).unwrap();
        assert!(
            latex.contains("\\multicolumn{2}{c}{Both} & C \\\\"),
            "{latex}"
        );
        assert!(latex.contains("x & 1 & 2 \\\\"), "{latex}");
        assert!(latex.contains(" & 3 & 4 \\\\"), "{latex}");
    }

    #[test]
    fn a_table_with_no_cells_is_not_a_table() {
        assert!(to_latex("<table></table>", &plain).is_none());
    }
}
