//! makes a set of cases from a seed.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{
    corpus::corpus_cases,
    document::DocGen,
    formula::FormulaGen,
    manifest::{Case, Grading, Manifest},
    render::{Rendered, degrade, render_latex},
    rng::Rng,
    util::{par_map, tools_dir},
    web::render_katex_cases,
};

type Pool = &'static [(&'static str, u32)];

const FONTS_ALL: Pool = &[
    ("latex-cm", 52),
    ("latex-lmodern", 14),
    ("xelatex-stix", 24),
    ("xelatex-lm", 10),
];
const FONTS_PDFLATEX: Pool = &[
    ("latex-cm", 60),
    ("latex-lmodern", 20),
    ("xelatex-stix", 20),
];
const FONTS_DOCUMENT: Pool = &[("latex-cm", 65), ("xelatex-stix", 35)];
const WEB: Pool = &[("katex", 1)];

/// name, weight in the mix, how it's graded, which renderers draw it.
const CATEGORIES: [(&str, u32, Grading, Pool); 10] = [
    ("formula-simple", 14, Grading::Formula, FONTS_ALL),
    ("formula-medium", 18, Grading::Formula, FONTS_ALL),
    ("formula-hard", 14, Grading::Formula, FONTS_ALL),
    ("matrix", 7, Grading::Formula, FONTS_PDFLATEX),
    ("cases", 4, Grading::Formula, FONTS_PDFLATEX),
    ("aligned", 4, Grading::Formula, FONTS_PDFLATEX),
    ("sentence", 10, Grading::Document, FONTS_DOCUMENT),
    ("document", 13, Grading::Document, FONTS_DOCUMENT),
    ("web-formula", 6, Grading::Formula, WEB),
    ("web-sentence", 3, Grading::Document, WEB),
];

fn category(name: &str) -> &'static (&'static str, u32, Grading, Pool) {
    CATEGORIES
        .iter()
        .find(|c| c.0 == name)
        .expect("a known category")
}

/// make one case, web cases are only described and rendered together later, None if it didn't compile or came out blank.
fn make_case(seed: u64, index: usize, category_name: &str, out_dir: &Path) -> Option<Case> {
    let (_, _, grading, pool) = category(category_name);
    let case_seed = format!("{seed}-{index}");
    let mut rng = Rng::from_text(&format!("{case_seed}-{category_name}"));
    let renderer = (*rng.weighted(pool)).to_string();
    let width_cm = *rng.pick(&[10u32, 12, 14]);
    let dpi = *rng.pick(&[150u32, 200, 250, 300]);
    let pt = *rng.pick(&[11u32, 12]);
    let truth = match category_name {
        "sentence" | "web-sentence" => DocGen::new(&mut rng).paragraph(),
        "document" => DocGen::new(&mut rng).document(),
        "web-formula" => FormulaGen::new(&mut rng).formula("formula-medium"),
        formula => FormulaGen::new(&mut rng).formula(formula),
    };
    let name = format!("{category_name}-{index:04}");
    let mut case = Case {
        name: name.clone(),
        category: category_name.to_string(),
        kind: *grading,
        truth,
        renderer: renderer.clone(),
        image: format!("{name}.png"),
        dpi,
        width_cm,
        seed: case_seed,
        degraded: None,
        theme: None,
        font_px: None,
        family: None,
        preamble: None,
        source: None,
    };
    if renderer == "katex" {
        case.theme = Some((*rng.pick(&["light", "light", "dark"])).to_string());
        case.font_px = Some(*rng.pick(&[20u32, 26, 32]));
        return Some(case);
    }
    let png = out_dir.join(&case.image);
    match render_latex(
        &case.truth,
        *grading,
        &renderer,
        dpi,
        &png,
        width_cm,
        pt,
        "",
        60,
    ) {
        Ok(Rendered::Done) => {}
        Ok(Rendered::Failed(why)) => {
            let shown: String = case.truth.chars().take(110).collect();
            eprintln!(
                "  skipped {name} ({renderer}): {why}\n    {}",
                shown.replace('\n', " ")
            );
            return None;
        }
        Err(e) => {
            eprintln!("  skipped {name}: {e:#}");
            return None;
        }
    }
    if rng.chance(0.25) {
        case.degraded = degrade(&png, &mut rng).ok().map(str::to_string);
    }
    Some(case)
}

/// the categories to make, drawn by weight.
fn plan(rng: &mut Rng, count: usize, kinds: Option<&BTreeSet<String>>) -> Vec<&'static str> {
    let allowed: Vec<(&'static str, u32)> = CATEGORIES
        .iter()
        .filter(|c| {
            kinds.is_none_or(|k| k.contains(c.0) || k.contains(c.0.split('-').next().unwrap_or("")))
        })
        .map(|c| (c.0, c.1))
        .collect();
    if allowed.is_empty() {
        return Vec::new();
    }
    (0..count).map(|_| *rng.weighted(&allowed)).collect()
}

/// where a run is kept.
pub fn default_dir(seed: u64, count: usize, kinds: Option<&BTreeSet<String>>) -> PathBuf {
    let tag = match kinds {
        Some(k) if !k.is_empty() => format!(
            "seed{seed}-n{count}-{}",
            k.iter().cloned().collect::<Vec<_>>().join("+")
        ),
        _ => format!("seed{seed}-n{count}"),
    };
    tools_dir().join("fixtures").join("generated").join(tag)
}

pub fn generate(
    seed: u64,
    count: usize,
    out_dir: &Path,
    kinds: Option<&BTreeSet<String>>,
    jobs: usize,
) -> Result<Manifest> {
    fs::create_dir_all(out_dir)?;
    // "corpus" cases come from real papers, run fetch-arxiv first.
    let want_corpus = kinds.is_some_and(|k| k.contains("corpus"));
    let others: Option<BTreeSet<String>> =
        kinds.map(|k| k.iter().filter(|x| *x != "corpus").cloned().collect());
    let only_corpus = want_corpus && others.as_ref().is_some_and(BTreeSet::is_empty);
    let n_corpus = if !want_corpus {
        0
    } else if only_corpus {
        count
    } else {
        (count / 4).max(1)
    };
    let n_other = count - n_corpus;

    let mut rng = Rng::from_text(&format!("plan-{seed}"));
    let planned = plan(&mut rng, n_other, others.as_ref());
    let items: Vec<(usize, &'static str)> = planned.into_iter().enumerate().collect();
    let total = items.len();
    let results = par_map(items, jobs, |(i, cat)| make_case(seed, i, cat, out_dir));
    let mut skipped = results.iter().filter(|r| r.is_none()).count();
    let mut cases: Vec<Case> = results.into_iter().flatten().collect();
    eprintln!("  rendered {total} (LaTeX cases are drawn; web cases are queued)");

    if n_corpus > 0 {
        cases.extend(corpus_cases(seed, n_corpus, out_dir, jobs));
    }
    let queued: Vec<usize> = (0..cases.len())
        .filter(|&i| cases[i].renderer == "katex")
        .collect();
    if !queued.is_empty() {
        let mut web: Vec<Case> = queued.iter().map(|&i| cases[i].clone()).collect();
        let ok = render_katex_cases(&mut web, out_dir, jobs)?;
        skipped += web.len() - ok.len();
        for (&i, updated) in queued.iter().zip(web) {
            cases[i] = updated;
        }
        cases.retain(|c| c.renderer != "katex" || ok.contains(&c.name));
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    let manifest = Manifest {
        seed,
        count,
        made: cases.len(),
        skipped,
        cases,
    };
    fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    eprintln!(
        "made {} cases in {} ({skipped} did not render and were skipped)",
        manifest.made,
        out_dir.display()
    );
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_follows_the_weights_and_the_allowed_kinds() {
        let mut rng = Rng::from_text("plan-test");
        let all = plan(&mut rng, 2000, None);
        let formulas = all.iter().filter(|c| c.starts_with("formula")).count();
        assert!(
            (800..1300).contains(&formulas),
            "formulas should be about half, got {formulas}"
        );
        let only: BTreeSet<String> = ["matrix".to_string()].into();
        assert!(
            plan(&mut rng, 20, Some(&only))
                .iter()
                .all(|c| *c == "matrix")
        );
        let web: BTreeSet<String> = ["web".to_string()].into();
        assert!(
            plan(&mut rng, 20, Some(&web))
                .iter()
                .all(|c| c.starts_with("web-"))
        );
        let nothing: BTreeSet<String> = ["nope".to_string()].into();
        assert!(plan(&mut rng, 5, Some(&nothing)).is_empty());
    }

    #[test]
    fn a_case_description_depends_only_on_the_seed_and_index() {
        // the web branch needs no rendering so the whole description can be compared.
        let dir = std::env::temp_dir();
        let a = make_case(7, 3, "web-formula", &dir).unwrap();
        let b = make_case(7, 3, "web-formula", &dir).unwrap();
        assert_eq!((a.truth, a.theme, a.dpi), (b.truth, b.theme, b.dpi));
        let c = make_case(7, 4, "web-formula", &dir).unwrap();
        assert_eq!(a.name, "web-formula-0003");
        assert_ne!(a.seed, c.seed);
    }

    #[test]
    fn runs_are_kept_in_a_folder_named_after_the_seed_count_and_kinds() {
        assert!(default_dir(7, 200, None).ends_with("generated/seed7-n200"));
        let k: BTreeSet<String> = ["web".to_string(), "corpus".to_string()].into();
        assert!(default_dir(7, 60, Some(&k)).ends_with("generated/seed7-n60-corpus+web"));
    }
}
