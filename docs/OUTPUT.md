# What texcopy writes

texcopy gives you LaTeX that **renders the same as the picture**. It can't know what the author actually typed (the picture can't tell `\textbf{x}` from `{\bf x}`), so what you get is a correct reconstruction, not a copy. This page covers what each kind of content turns into.

Point it at a line or two from a textbook problem and you get running text with the math marked:

```
$ texcopy problem.png
5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$.
```

With the vision engine, a whole page of text works too:

```
$ texcopy page.png
\section*{Introduction}
Today I am learning \LaTeX{}. I can write in line math such as $a^2 + b^2 = c^2$.

I can also give equations their own space:
\begin{equation}
\gamma^2+\theta^2=\omega^2
\end{equation}
```

- **Display equations** stay display equations. With a number next to them (`(1)`, or a `\tag`) they become `\begin{equation}...\end{equation}` and LaTeX writes the number itself. Without one they become `\[ ... \]`. `align`-style environments are kept as they are. If a page number is left at the very end after one, it's dropped.
- **Paragraphs** keep their blank line. Lines that were wrapped inside a paragraph get joined.
- **Headings** (`##`) become `\section*{...}` (`###` is `\subsection*`). The model marks centred text as a heading too, and texcopy can't tell the difference.
- **Lists** become `itemize` or `enumerate`, nested when the bullets change. A lone "5." is a problem number, not a list.
- **Text is made safe for LaTeX.** `& % # _` and prices (`\$5`) get escaped, `"..."` and curly quotes become ` ``...'' `, dashes become `--` and `---`, a `□` ends a proof flush right, and the drawn `LATEX` and `TEX` logos become `\LaTeX{}` and `\TeX{}`.
- **Code** in a fenced block becomes `verbatim`. If the model writes `**bold**` or `*italic*` Markdown, those become `\textbf` and `\emph`.
- **Not recoverable:** the model usually gives back bold, italic, small caps, underline and typewriter text as plain words, and it doesn't report vertical table rules, so those come out plain.

## Human mode and exact mode

By default texcopy is in **human** mode. It cleans the model's output up so it reads like LaTeX someone typed:

- `x^{2}` and `K_{2}` become `x^2` and `K_2`.
- The braces the model puts around every cell in `aligned`, `array`, `gathered`, `cases` and matrices go away, and a `{}=` after an `&` becomes `&=`. The column spec of an `array` stays.
- `\;` and `\:` between plain symbols are dropped. Next to a command or a group they stay, because there they can be doing real work.
- Spacing macros at the very start or end are removed. The small model reads an equation number like `(1)` as a long run of `\qquad`, and that's what this catches.

None of that changes what gets typeset, apart from the spacing bits.

**Exact** mode skips all of it and gives you the model's text character for character. It's for debugging, or when you want to see what the model actually said. Pick it with `--mode exact` (`--raw` does the same), `TEXCOPY_MODE=exact`, or the **mode** buttons in the window. The window remembers your choice, and switching doesn't read the picture again.

`--json` always has both: `latex` follows the mode you picked, and `raw` is the model's text.

## Tables

With the vision engine, a table comes back as a `tabular`:

```
\begin{center}
\begin{tabular}{rcc}
\hline
$n$ & Recursive time (ms) & DP time (ms) \\
\hline
5 & 0.000037 & 0.000022 \\
...
\hline
\end{tabular}
\end{center}
```

The model reads the cells, but the layout is a guess, because its HTML table doesn't say anything about alignment or rules. Number-like columns (`12`, `0.5`, `$1,250.00`, `50%`) are right-aligned and text columns are left-aligned. There's a rule above, below and under a header row, and no vertical lines. `colspan` becomes `\multicolumn`. `rowspan` leaves the covered cells empty (which row of the span the text belongs in is ambiguous). A single letter in the header row (`n`) is a variable, so it becomes `$n$`. In the body it's a label (a grade like `C`) and stays text. Cells get cleaned like any other text, so `&`, `%`, `_` and prices are escaped. Change `{lrr}` and the rules to taste.

Sometimes the model writes a table as plain lines instead (`Id Name Year` / `1 Ann 2021` ...). Three or more lines with the same number of short words and a column of numbers get rebuilt as a table too. A line ending in a colon is a caption, not a row.

The tables benchmark (`tools/bench.py --suite tables`) grades the *cells*, not how it looks: 16 of 18 exact. The two misses are a `rowspan` whose text sits in the other row, and `g/cm$^3$` that the model reads as `g/cm3`.
