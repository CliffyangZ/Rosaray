//! SC-007 as a property: for generated method passages with omitted units,
//! values and formulas, every field the proposer emits is either anchored in its
//! source quote or absent with an ambiguity — there is no defaulting path.

use rosaray_service::designer::paper::candidate::{CandidateProposer, Item, ProposedCandidate};
use rosaray_service::designer::paper::pdf::PageText;
use rosaray_service::designer::paper::rules::RulesProposer;
use serde_json::Value;

/// Small deterministic generator (no external property-testing dependency).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() as usize) % xs.len()]
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

fn items(c: &ProposedCandidate) -> Vec<&Item> {
    let p = &c.proposed;
    p.inputs.iter().chain(&p.outputs).chain(&p.parameters).chain(&p.units).chain(&p.assumptions).chain(p.formula.as_ref()).collect()
}

fn number_text(v: &Value) -> Option<String> {
    v.as_f64().map(|f| if f.fract() == 0.0 { format!("{}", f as i64) } else { format!("{f}") })
}

fn assert_anchored(c: &ProposedCandidate, passage: &str) {
    for it in items(c) {
        if it.value.is_none() && it.unit.is_none() {
            continue;
        }
        let span = it.source_span.as_ref().unwrap_or_else(|| panic!("unanchored {it:?} from {passage:?}"));
        let quote = span.quote.to_ascii_lowercase();
        if let Some(u) = &it.unit {
            assert!(quote.contains(&u.to_ascii_lowercase()), "unit {u:?} is not in its quote {quote:?}");
        }
        if let Some(v) = &it.value {
            if let Some(n) = number_text(v) {
                assert!(quote.contains(&n), "value {n} is not in its quote {quote:?}");
            } else if let Some(s) = v.as_str() {
                let head: String = s.split_whitespace().take(3).collect::<Vec<_>>().join(" ").to_ascii_lowercase();
                assert!(quote.contains(&head), "value {s:?} is not in its quote {quote:?}");
            }
        }
    }
}

fn run(passage: &str) -> Vec<ProposedCandidate> {
    RulesProposer.propose(&[PageText { page: 1, text: format!("1. Methods\n{passage}\n") }])
}

#[test]
fn parameters_are_anchored_or_absent_with_an_ambiguity() {
    let mut g = Lcg(20260926);
    let ops = [("A Gaussian filter", "sigma"), ("A global threshold", "threshold"), ("Morphological opening", "radius")];
    let units = ["pixels", "mm", "intensity units", "percent"];
    for _ in 0..600 {
        let (op, param) = *g.pick(&ops);
        let has_number = g.chance(60);
        let has_unit = has_number && g.chance(50);
        let n = (g.next() % 200) as f64 / if g.chance(30) { 10.0 } else { 1.0 };
        let number = if n.fract() == 0.0 { format!("{}", n as i64) } else { format!("{n}") };
        let unit = g.pick(&units);
        let passage = match (has_number, has_unit) {
            (true, true) => format!("{op} with {param} = {number} {unit} was applied to the images."),
            (true, false) => format!("{op} with {param} = {number} was applied to the images."),
            (false, _) => format!("{op} was applied to the images."),
        };
        let candidates = run(&passage);
        assert!(!candidates.is_empty(), "{passage}");
        for c in &candidates {
            assert_anchored(c, &passage);
            let named: Vec<&Item> = c.proposed.parameters.iter().filter(|p| p.name == param).collect();
            if has_number && matches!(param, "sigma" | "radius") || has_number && param == "threshold" {
                // A stated number is captured exactly, and only with the unit the source gave.
                if let Some(p) = named.first() {
                    assert_eq!(p.unit.is_some(), has_unit, "{passage}");
                    if !has_unit {
                        assert!(c.ambiguities.iter().any(|a| a.code == "missing_unit" && a.field == param), "{passage}");
                    }
                }
            }
            if !has_number {
                assert!(named.is_empty(), "a value was invented for {param}: {passage}");
                assert!(c.ambiguities.iter().any(|a| a.code == "parameter_value_absent"), "{passage}");
            }
        }
    }
}

#[test]
fn formulas_are_never_invented() {
    let mut g = Lcg(7);
    let variants = [
        ("The area was computed as the pixel count multiplied by the squared spacing.", false),
        ("The margin ratio is calculated using the method of Smith et al.", false),
        ("Formula: A = N x s^2 where N is the number of pixels.", true),
        ("The ratio r = a / b was reported.", true),
    ];
    for _ in 0..200 {
        let (text, written) = *g.pick(&variants);
        for c in run(text) {
            assert_anchored(&c, text);
            match &c.proposed.formula {
                Some(f) => {
                    assert!(written, "invented a formula for {text:?}: {f:?}");
                    assert!(f.source_span.is_some());
                }
                None => {
                    assert!(!written, "missed a written formula in {text:?}");
                    assert!(c.ambiguities.iter().any(|a| a.code == "formula_unparsed"), "{text}");
                }
            }
        }
    }
}

#[test]
fn nothing_with_a_value_ever_lacks_a_source_across_random_prose() {
    let mut g = Lcg(99);
    let subjects = ["The images", "Photographs", "A 3 mm margin", "Gingival area", "The mask", "Patients"];
    let verbs = ["were normalized", "was computed as 12 pixels", "were excluded", "was evaluated on 40 images", "was thresholded above 0.6", "were smoothed with sigma = 1.5"];
    for _ in 0..500 {
        let passage = format!("{} {}. {} {}.", g.pick(&subjects), g.pick(&verbs), g.pick(&subjects), g.pick(&verbs));
        for c in run(&passage) {
            assert_anchored(&c, &passage);
            assert!(!c.sources.is_empty(), "{passage}");
            for s in &c.sources {
                assert!(!s.quote.is_empty() && s.page == 1);
            }
        }
    }
}
