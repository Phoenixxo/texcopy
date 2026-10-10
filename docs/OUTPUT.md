# What texcopy writes

texcopy returns LaTeX that **renders the same as the picture**. It cannot know the author's original source, so the result is a correct reconstruction and not a copy. This page lists what each kind of content becomes.

Point it at a line or two of a textbook problem and it returns running text with the math marked:

```
$ texcopy problem.png
5.4 Prove that if $v$ is a cut-vertex of a graph $G$, then $v$ is not a cut-vertex of the complement $\overline{G}$ of $G$.
```

With the vision engine, a page of text works too. texcopy rebuilds it as LaTeX that **renders the same as the picture**; it cannot know the author's original source (the picture can't tell `\textbf{x}` from `{\bf x}`), so the result is a correct reconstruction, not a copy:

```
$ texcopy page.png
\section*{Introduction}
Today I am learning \LaTeX{}. I can write in line math such as $a^2 + b^2 = c^2$.

I can also give equations their own space:
\begin{equation}
\gamma^{2}+\theta^{2}=\omega^{2}
\end{equation}
```

- **Display equations** are display equations. With a number beside them (`(1)`, or a `\tag`) they become `\begin{equation}...\end{equation}` (LaTeX writes the number itself); without one they become `\[ ... \]`; `align`-type environments are kept as they are. A page number left at the very end after one is dropped.
- **Paragraphs** keep their blank line; lines wrapped inside a paragraph are joined.
- **Headings** (`##`) become `\section*{...}` (`###` is `\subsection*`). The model marks centred text as a heading too, and texcopy can't tell the difference.
- **Lists** become `itemize` / `enumerate`, nested when the bullets change. A lone "5." is a problem number, not a list.
- **Text is made safe for LaTeX:** `& % # _` and prices (`\$5`) are escaped, `"..."` and curly quotes become ` ``...'' `, dashes become `--` and `---`, a `□` ends a proof flush right, and the drawn `LATEX`/`TEX` logos become `\LaTeX{}`/`\TeX{}`.
- **Code** in a fenced block becomes `verbatim`; `**bold**` and `*italic*` Markdown, if the model writes any, become `\textbf` and `\emph`.
- **Not recoverable:** the model usually returns bold, italic, small caps, underline and typewriter text as plain words, and does not report vertical table rules, so those come out plain.

**Tables** (vision engine) come back as a `tabular`:

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

The model reads the cells; the layout is guessed, because the model's HTML table says nothing about alignment or rules. Number-like columns (`12`, `0.5`, `$1,250.00`, `50%`) are right-aligned and text columns left-aligned; there is a rule above, below, and under a header row, and no vertical lines. `colspan` becomes `\multicolumn`; `rowspan` leaves the covered cells empty (which row of the span the text belongs in is ambiguous). A single letter in the header row (`n`) is a variable, so it becomes `$n$`; in the body it is a label (a grade `C`) and stays text. Cells are cleaned like any other text: `&`, `%`, `_` and prices are escaped. Adjust `{lrr}` and the rules to taste.

Sometimes the model writes a table as plain lines instead (`Id Name Year` / `1 Ann 2021` ...). Three or more lines of the same number of short words with a column of numbers are rebuilt as a table too; a line ending in a colon is a caption, not a row.

The tables benchmark (`tools/bench.py --suite tables`) grades the *cells*, not the look: 16 of 18 exact. The two misses are a `rowspan` whose text sits in the other row, and `g/cm$^3$` that the model reads as `g/cm3`.
