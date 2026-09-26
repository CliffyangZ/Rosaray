//! Rules-based `CandidateProposer` (research §12): section detection, sentence
//! segmentation, a category lexicon and `number + unit` patterns. Deterministic
//! and offline. Recall is modest by design — SC-006 measures reviewability, not
//! extraction quality — and it never emits a value without a source span.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::json;

use super::candidate::{Ambiguity, Category, CandidateProposer, Item, Proposed, ProposedCandidate, SourceSpan};
use super::pdf::PageText;

pub struct RulesProposer;

// ---- segmentation -----------------------------------------------------------

struct Sentence {
    page: u32,
    section: Option<String>,
    start: usize,
    end: usize,
    /// The sentence with line breaks folded to single spaces.
    text: String,
}

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*\d+(\.\d+)*\.?\s+[A-Z][^.]{0,80}$").unwrap())
}

const ABBREVIATIONS: &[&str] = &["al", "e.g", "i.e", "fig", "eq", "vs", "approx", "no", "dr"];

/// Splits one paragraph slice into sentences (byte offsets into `text`).
fn split_sentences(text: &str, base: usize) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if matches!(c, b'.' | b'!' | b'?') {
            // A boundary needs whitespace (or end) after it and, unless it ends the text,
            // an uppercase letter or digit-led heading after that.
            let after = &text[i + 1..];
            let ws = after.chars().next().map(char::is_whitespace).unwrap_or(true);
            let next_upper = after.trim_start().chars().next().map(|n| n.is_uppercase() || n.is_ascii_digit()).unwrap_or(true);
            let before_word = text[start..i].rsplit(|c: char| c.is_whitespace()).next().unwrap_or("").trim_start_matches('(');
            let abbreviation = ABBREVIATIONS.contains(&before_word.to_ascii_lowercase().as_str());
            let decimal = c == b'.' && i > 0 && bytes[i - 1].is_ascii_digit() && after.chars().next().is_some_and(|n| n.is_ascii_digit());
            // A line break right after the full stop ends the sentence even after
            // an abbreviation ("… Smith et al.⏎Pixels …").
            let line_break = after.chars().take_while(|c| c.is_whitespace()).any(|c| c == '\n');
            if ws && (next_upper || line_break) && (!abbreviation || line_break) && !decimal {
                out.push((base + start, base + i + 1));
                start = i + 1;
            }
        }
        i += 1;
    }
    if start < text.len() {
        out.push((base + start, base + text.len()));
    }
    out.into_iter()
        .filter_map(|(s, e)| {
            let slice = &text[s - base..e - base];
            let lead = slice.len() - slice.trim_start().len();
            let trimmed = slice.trim();
            (!trimmed.is_empty()).then_some((s + lead, s + lead + trimmed.len()))
        })
        .collect()
}

fn sentences_of(pages: &[PageText]) -> Vec<Sentence> {
    let mut out = Vec::new();
    let mut section: Option<String> = None;
    for p in pages {
        // Walk the page line by line: headings set the section, other runs of
        // lines form paragraphs that are split into sentences.
        let mut para_start: Option<usize> = None;
        let mut para_end = 0usize;
        let mut offset = 0usize;
        let flush = |start: Option<usize>, end: usize, section: &Option<String>, out: &mut Vec<Sentence>| {
            if let Some(s) = start {
                let slice = &p.text[s..end];
                // A lone unpunctuated line is a title or caption, not a method statement.
                if !slice.contains('\n') && !slice.trim_end().ends_with(['.', '!', '?']) {
                    return;
                }
                for (a, b) in split_sentences(slice, s) {
                    let text: String = p.text[a..b].split_whitespace().collect::<Vec<_>>().join(" ");
                    out.push(Sentence { page: p.page, section: section.clone(), start: a, end: b, text });
                }
            }
        };
        let lines: Vec<&str> = p.text.split_inclusive('\n').collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_end_matches(['\n', '\r']);
            let next_starts_upper = lines.get(i + 1).and_then(|l| l.trim_start().chars().next()).is_some_and(|c| c.is_uppercase());
            // An unpunctuated short line followed by a capitalised line is a title or
            // caption: it ends the previous paragraph and forms none of its own.
            let caption = !trimmed.trim().is_empty()
                && trimmed.len() <= 100
                && !trimmed.trim_end().ends_with(['.', '!', '?', ':', ';', ','])
                && next_starts_upper
                && !heading_re().is_match(trimmed);
            if heading_re().is_match(trimmed) && !trimmed.ends_with('.') {
                flush(para_start.take(), para_end, &section, &mut out);
                section = Some(trimmed.trim().to_string());
            } else if caption {
                flush(para_start.take(), para_end, &section, &mut out);
            } else if trimmed.trim().is_empty() {
                flush(para_start.take(), para_end, &section, &mut out);
            } else {
                if para_start.is_none() {
                    para_start = Some(offset);
                }
                para_end = offset + trimmed.len();
            }
            offset += line.len();
        }
        flush(para_start.take(), para_end, &section, &mut out);
    }
    out
}

// ---- classification ------------------------------------------------------------

fn re(pattern: &str) -> Regex {
    Regex::new(&format!("(?i){pattern}")).expect("static pattern is valid")
}

struct Lexicon {
    clinical: Regex,
    validation: Regex,
    eligibility: Regex,
    measurement: Regex,
    postprocessing: Regex,
    preprocessing: Regex,
    inference: Regex,
    landmark: Regex,
    decision: Regex,
}

fn lexicon() -> &'static Lexicon {
    static L: OnceLock<Lexicon> = OnceLock::new();
    L.get_or_init(|| Lexicon {
        clinical: re(r"\b(diagnos(?:e|es|is|ed|tic)|treatment decisions?|should guide (?:the )?treatment|recommend(?:s|ed)? (?:the )?(?:treatment|therapy)|prognos\w+)\b"),
        validation: re(r"\b(evaluated|validation|validated|dice|iou|sensitivity|specificity|accuracy|manual annotation|ground truth|reference standard|test set|cross-?validation)\b"),
        eligibility: re(r"\b(excluded|exclusion|included|inclusion|eligib\w+|only (?:photographs|images|patients))\b"),
        measurement: re(r"\b(area|length|distance|width|volume|ratio|pixel count|computed as|calculated|formula)\b"),
        postprocessing: re(r"\b(morpholog\w+|opening|closing|erosion|dilation|connected components?|post-?process\w*)\b"),
        preprocessing: re(r"\b(normaliz\w+|grayscale|gaussian|blur\w*|denois\w+|filter\w*|resiz\w+|crop\w*|contrast|histogram|stretch\w*)\b"),
        inference: re(r"\b(threshold\w*|segment\w*|neural network|deep learning|cnn|u-?net|classifier|inference|trained)\b"),
        landmark: re(r"\b(landmarks?|keypoints?|fiducials?)\b"),
        decision: re(r"\b(cut-?off|classified as|criterion|criteria for|considered (?:positive|abnormal))\b"),
    })
}

/// The topic keyword that identifies "the same step" for merging.
fn topic_of(category: Category, text: &str) -> Option<String> {
    let l = text.to_ascii_lowercase();
    let table: &[&str] = match category {
        Category::Preprocessing => &["gaussian", "normaliz", "grayscale", "denois", "resiz", "crop", "contrast", "histogram"],
        Category::ModelInference => &["threshold", "segment", "neural", "classifier", "u-net", "unet"],
        Category::Postprocessing => &["morpholog", "opening", "closing", "erosion", "dilation", "connected"],
        Category::Measurement => &["area", "length", "distance", "width", "volume", "ratio"],
        _ => &[],
    };
    table.iter().find(|k| l.contains(*k)).map(|k| k.to_string())
}

fn classify(text: &str) -> Option<Category> {
    let lx = lexicon();
    if lx.clinical.is_match(text) {
        Some(Category::ClinicalClaim)
    } else if lx.validation.is_match(text) {
        Some(Category::ValidationOnly)
    } else if lx.eligibility.is_match(text) {
        Some(Category::DataEligibility)
    } else if lx.measurement.is_match(text) {
        Some(Category::Measurement)
    } else if lx.postprocessing.is_match(text) {
        Some(Category::Postprocessing)
    } else if lx.preprocessing.is_match(text) {
        Some(Category::Preprocessing)
    } else if lx.landmark.is_match(text) {
        Some(Category::LandmarkExtraction)
    } else if lx.inference.is_match(text) || threshold_phrase().is_match(text) {
        Some(Category::ModelInference)
    } else if lx.decision.is_match(text) {
        Some(Category::DecisionRule)
    } else {
        None
    }
}

// ---- field extraction ------------------------------------------------------------

fn threshold_phrase() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| re(r"\b(above|below|greater than|less than|exceeding)\s+-?\d"))
}

const UNITS: &str = r"(?:mm2|mm²|mm|cm|px|pixels?|percentiles?|percent|%|intensity units?|intensity|degrees?|°|hz|ms|images?|photographs?|samples?)";

/// `name = 2`, `name of 128 intensity units`, `radius of 3 pixels` …
fn named_number() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        re(&format!(
            r"\b(sigma|σ|radius|threshold|window|kernel size|size|alpha|beta|gamma|pixel spacing|spacing|lambda)\s*(?:=|:|of|at)?\s*(-?\d+(?:\.\d+)?)(?:\s*({UNITS})\b)?"
        ))
    })
}

fn threshold_number() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| re(&format!(r"\b(?:above|below|greater than|less than|exceeding)\s+(-?\d+(?:\.\d+)?)(?:\s*({UNITS})\b)?")))
}

fn percentile_pair() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| re(r"\bbetween the (\d+)(?:st|nd|rd|th) and (\d+)(?:st|nd|rd|th) percentiles?"))
}

fn counted() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| re(r"\b(\d+)\s+(images?|photographs?|samples?)\b"))
}

fn formula_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:Formula:\s*)?\b([A-Za-z][A-Za-z0-9_]{0,12})\s*=\s*([A-Za-z0-9_ ()^*/+\-×]*[\^*/+×\-]|[A-Za-z0-9_]+ x [A-Za-z0-9_^]+)[A-Za-z0-9_ ^()*/+\-×]*").unwrap())
}

fn parameter_words() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| re(r"\b(gaussian|threshold|morpholog\w+|opening|closing|erosion|dilation)\b"))
}

fn span(s: &Sentence, quote_range: Option<(usize, usize)>) -> SourceSpan {
    // Quote is the whole sentence: the researcher reviews context, not a fragment.
    let _ = quote_range;
    SourceSpan { page: s.page, section: s.section.clone(), quote: s.text.clone(), start: s.start, end: s.end }
}

fn contains(hay: &str, needle: &str) -> bool {
    hay.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

fn extract_fields(category: Category, s: &Sentence) -> (Proposed, Vec<Ambiguity>) {
    let mut proposed = Proposed::default();
    let mut ambiguities = Vec::new();
    let sp = span(s, None);
    let text = &s.text;

    if category == Category::ClinicalClaim {
        return (proposed, ambiguities);
    }

    // Parameters with their own numbers.
    for c in named_number().captures_iter(text) {
        let name = c[1].to_ascii_lowercase().replace('σ', "sigma");
        let num: f64 = c[2].parse().unwrap_or(0.0);
        let unit = c.get(3).map(|m| m.as_str().to_string());
        if unit.is_none() {
            ambiguities.push(Ambiguity {
                code: "missing_unit".into(),
                field: name.clone(),
                note: format!("The source gives {name} = {} without a unit.", &c[2]),
            });
        }
        proposed.parameters.push(Item { name, value: Some(json!(num)), unit, source_span: Some(sp.clone()) });
    }
    for c in threshold_number().captures_iter(text) {
        let num: f64 = c[1].parse().unwrap_or(0.0);
        let unit = c.get(2).map(|m| m.as_str().to_string());
        if unit.is_none() {
            ambiguities.push(Ambiguity {
                code: "missing_unit".into(),
                field: "threshold".into(),
                note: format!("The source keeps pixels above {} but does not say in what unit or scale.", &c[1]),
            });
        }
        proposed.parameters.push(Item { name: "threshold".into(), value: Some(json!(num)), unit, source_span: Some(sp.clone()) });
    }
    if let Some(c) = percentile_pair().captures(text) {
        for (i, name) in [(1, "lower_percentile"), (2, "upper_percentile")] {
            proposed.parameters.push(Item {
                name: name.into(),
                value: Some(json!(c[i].parse::<f64>().unwrap_or(0.0))),
                unit: Some("percentile".into()),
                source_span: Some(sp.clone()),
            });
        }
    }
    if let Some(c) = counted().captures(text) {
        proposed.parameters.push(Item {
            name: "sample_size".into(),
            value: Some(json!(c[1].parse::<f64>().unwrap_or(0.0))),
            unit: Some(c[2].to_ascii_lowercase()),
            source_span: Some(sp.clone()),
        });
    }

    // A parameterised operation named without any value.
    if let Some(m) = parameter_words().find(text) {
        let word = m.as_str().to_ascii_lowercase();
        let (field, present) = if word == "gaussian" {
            ("sigma", proposed.parameters.iter().any(|p| p.name == "sigma"))
        } else if word == "threshold" {
            ("threshold", proposed.parameters.iter().any(|p| p.name == "threshold"))
        } else {
            ("radius", proposed.parameters.iter().any(|p| p.name == "radius"))
        };
        if !present {
            ambiguities.push(Ambiguity {
                code: "parameter_value_absent".into(),
                field: field.into(),
                note: format!("The source names the {word} step but gives no value for {field}."),
            });
        }
    }

    // Formulas: only an explicit expression is proposed; a computation described in words is an ambiguity.
    if let Some(m) = formula_re().find(text) {
        let raw = m.as_str().trim_start_matches("Formula:").trim();
        let cut = raw.find(" where ").or_else(|| raw.find([',', ';'])).unwrap_or(raw.len());
        let expr = raw[..cut].trim().to_string();
        proposed.formula = Some(Item { name: "formula".into(), value: Some(json!(expr)), unit: None, source_span: Some(sp.clone()) });
    } else if category == Category::Measurement && re(r"\b(computed|calculated)\b").is_match(text) {
        ambiguities.push(Ambiguity {
            code: "formula_unparsed".into(),
            field: "formula".into(),
            note: "A computation is described but no explicit formula is given; it is not guessed.".into(),
        });
    }

    // Inputs and outputs only when the sentence names them.
    let mentions_image = re(r"\b(images?|photographs?)\b").is_match(text);
    let mentions_mask = contains(text, "mask");
    let producing = re(r"\b(obtain|produce|generate|yield)\w*\b[^.]*\bmask\b").is_match(text);
    if mentions_image && !producing {
        proposed.inputs.push(Item { name: "image".into(), value: None, unit: None, source_span: Some(sp.clone()) });
    }
    if mentions_mask {
        let item = Item { name: "mask".into(), value: None, unit: None, source_span: Some(sp.clone()) };
        if producing {
            proposed.outputs.push(item);
        } else {
            proposed.inputs.push(item);
        }
    }
    if proposed.inputs.is_empty() && proposed.outputs.is_empty() {
        ambiguities.push(Ambiguity {
            code: "unstated_io".into(),
            field: "inputs/outputs".into(),
            note: "The source sentence does not say what this step takes in or produces.".into(),
        });
    }

    // Classical thresholding has no exact category in the fixed list.
    if category == Category::ModelInference && !re(r"\b(neural|deep learning|cnn|u-?net|classifier|trained|inference)\b").is_match(text) {
        ambiguities.push(Ambiguity {
            code: "category_uncertain".into(),
            field: "category".into(),
            note: "Classical thresholding/segmentation is filed under model_inference for lack of a closer category; review.".into(),
        });
    }
    (proposed, ambiguities)
}

impl CandidateProposer for RulesProposer {
    fn propose(&self, pages: &[PageText]) -> Vec<ProposedCandidate> {
        let sentences = sentences_of(pages);
        let mut out: Vec<(ProposedCandidate, Option<String>)> = Vec::new();
        for s in &sentences {
            let Some(category) = classify(&s.text) else { continue };
            let (proposed, ambiguities) = extract_fields(category, s);
            let topic = topic_of(category, &s.text);
            let candidate = ProposedCandidate { category, sources: vec![span(s, None)], proposed, ambiguities };

            // Several passages about one step become one candidate with several sources (US5-4).
            if let Some(t) = &topic {
                if let Some((existing, _)) = out.iter_mut().find(|(c, k)| c.category == category && k.as_ref() == Some(t)) {
                    merge(existing, candidate);
                    continue;
                }
            }
            out.push((candidate, topic));
        }

        let has_preprocessing = out.iter().any(|(c, _)| c.category == Category::Preprocessing);
        for (c, _) in out.iter_mut() {
            if matches!(c.category, Category::ModelInference | Category::Measurement) && !has_preprocessing {
                c.ambiguities.push(Ambiguity {
                    code: "unstated_preprocessing".into(),
                    field: "preprocessing".into(),
                    note: "No preprocessing before this step is described anywhere in the paper.".into(),
                });
            }
        }
        out.into_iter().map(|(c, _)| c).collect()
    }
}

fn merge(into: &mut ProposedCandidate, other: ProposedCandidate) {
    into.sources.extend(other.sources);
    let mut keep = |items: &mut Vec<Item>, add: Vec<Item>| {
        for it in add {
            match items.iter().find(|x| x.name == it.name) {
                Some(x) if x.value == it.value && x.unit == it.unit => {} // the same statement, seen twice
                Some(_) => {
                    into.ambiguities.push(Ambiguity {
                        code: "conflicting_values".into(),
                        field: it.name.clone(),
                        note: format!("Different passages give different values for {}.", it.name),
                    });
                    items.push(it);
                }
                None => items.push(it),
            }
        }
    };
    keep(&mut into.proposed.parameters, other.proposed.parameters);
    keep(&mut into.proposed.inputs, other.proposed.inputs);
    keep(&mut into.proposed.outputs, other.proposed.outputs);
    keep(&mut into.proposed.assumptions, other.proposed.assumptions);
    if into.proposed.formula.is_none() {
        into.proposed.formula = other.proposed.formula;
    }
    for a in other.ambiguities {
        if !into.ambiguities.contains(&a) {
            into.ambiguities.push(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(text: &str) -> Vec<PageText> {
        vec![PageText { page: 1, text: text.to_string() }]
    }

    fn one(text: &str) -> ProposedCandidate {
        let mut c = RulesProposer.propose(&pages(text));
        assert_eq!(c.len(), 1, "{c:?}");
        c.remove(0)
    }

    #[test]
    fn sentences_keep_decimals_and_abbreviations_whole() {
        let p = pages("The pixel spacing was 0.05 mm. Smith et al. described it. Then done.");
        let s = sentences_of(&p);
        assert_eq!(s.len(), 3, "{:?}", s.iter().map(|x| &x.text).collect::<Vec<_>>());
        assert!(s[0].text.contains("0.05 mm"));
        assert!(s[1].text.starts_with("Smith et al."));
    }

    #[test]
    fn quote_offsets_point_at_the_original_text() {
        let text = "1. Methods\nImages were normalized\nby percentile stretching.\nA Gaussian filter with sigma = 2 pixels was applied.\n";
        let c = RulesProposer.propose(&pages(text));
        for cand in &c {
            for s in &cand.sources {
                let raw: String = text[s.start..s.end].split_whitespace().collect::<Vec<_>>().join(" ");
                assert_eq!(raw, s.quote);
                assert_eq!(s.section.as_deref(), Some("1. Methods"));
            }
        }
    }

    #[test]
    fn classifies_the_fixed_categories() {
        for (text, cat) in [
            ("Images with visible blur were excluded from the study.", Category::DataEligibility),
            ("Images were normalized before analysis.", Category::Preprocessing),
            ("Morphological opening was applied.", Category::Postprocessing),
            ("The gingival area was computed as the pixel count.", Category::Measurement),
            ("Performance was evaluated against manual annotation.", Category::ValidationOnly),
            ("This method diagnoses periodontitis.", Category::ClinicalClaim),
            ("Landmarks were placed on the cusp tips.", Category::LandmarkExtraction),
        ] {
            assert_eq!(one(text).category, cat, "{text}");
        }
    }

    #[test]
    fn a_number_with_its_unit_is_anchored_and_a_missing_unit_is_an_ambiguity() {
        let c = one("A radius of 3 pixels was used for the morphological opening.");
        let p = &c.proposed.parameters[0];
        assert_eq!((p.name.as_str(), p.value.clone(), p.unit.as_deref()), ("radius", Some(json!(3.0)), Some("pixels")));
        assert!(p.source_span.is_some());
        assert!(!c.ambiguities.iter().any(|a| a.code == "missing_unit"));

        let c = one("A Gaussian filter with sigma = 2 was applied.");
        let p = c.proposed.parameters.iter().find(|p| p.name == "sigma").unwrap();
        assert_eq!(p.unit, None, "the unit is not defaulted");
        assert!(c.ambiguities.iter().any(|a| a.code == "missing_unit" && a.field == "sigma"));
    }

    #[test]
    fn an_operation_without_a_value_is_parameter_value_absent_not_a_default() {
        let c = one("A Gaussian filter was applied to reduce noise.");
        assert!(c.proposed.parameters.is_empty());
        assert!(c.ambiguities.iter().any(|a| a.code == "parameter_value_absent" && a.field == "sigma"));
    }

    #[test]
    fn formulas_are_only_proposed_when_written_out() {
        let c = one("Formula: A = N x s^2 where N is the number of mask pixels.");
        assert!(c.proposed.formula.as_ref().unwrap().value.as_ref().unwrap().as_str().unwrap().starts_with("A = N x s^2"));
        let c = one("The gingival area was computed as the pixel count multiplied by the squared spacing.");
        assert!(c.proposed.formula.is_none());
        assert!(c.ambiguities.iter().any(|a| a.code == "formula_unparsed"));
    }

    #[test]
    fn passages_about_one_step_become_one_candidate_with_all_sources() {
        let p = vec![
            PageText { page: 1, text: "A Gaussian filter with sigma = 2 was applied.\n".into() },
            PageText { page: 2, text: "The Gaussian blur (sigma = 2) suppressed noise.\n".into() },
        ];
        let c = RulesProposer.propose(&p);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].sources.iter().map(|s| s.page).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(c[0].proposed.parameters.iter().filter(|p| p.name == "sigma").count(), 1, "identical statements are not duplicated");

        let p = vec![
            PageText { page: 1, text: "A Gaussian filter with sigma = 2 was applied.\n".into() },
            PageText { page: 2, text: "The Gaussian blur (sigma = 3) suppressed noise.\n".into() },
        ];
        let c = RulesProposer.propose(&p);
        assert!(c[0].ambiguities.iter().any(|a| a.code == "conflicting_values"));
    }

    #[test]
    fn missing_preprocessing_is_flagged_on_later_steps() {
        let c = one("A global threshold of 128 intensity units was applied to obtain the mask.");
        assert!(c.ambiguities.iter().any(|a| a.code == "unstated_preprocessing"));
        assert_eq!(c.category, Category::ModelInference);
        assert!(c.ambiguities.iter().any(|a| a.code == "category_uncertain"));
        assert_eq!(c.proposed.outputs[0].name, "mask");
    }

    #[test]
    fn a_title_line_and_an_et_al_sentence_end_do_not_leak_into_steps() {
        let text = "Automated Gingival Area Measurement from Photographs\nThe margin ratio is calculated using the method of Smith et al.\nPixels above 0.6 were retained.\n";
        let c = RulesProposer.propose(&pages(text));
        assert!(c.iter().all(|c| !c.sources.iter().any(|s| s.quote.contains("Automated"))), "{c:?}");
        let ratio = c.iter().find(|c| c.sources[0].quote.contains("margin ratio")).unwrap();
        assert_eq!(ratio.sources.len(), 1);
        assert!(ratio.proposed.parameters.is_empty(), "the 0.6 belongs to the next sentence");
        let kept = c.iter().find(|c| c.sources[0].quote.starts_with("Pixels above")).unwrap();
        assert_eq!(kept.proposed.parameters[0].value, Some(json!(0.6)));
        assert!(kept.ambiguities.iter().any(|a| a.code == "missing_unit"));
    }

    #[test]
    fn a_formula_stops_before_its_explanation() {
        let c = one("Formula: A = N x s^2 where N is the number of mask pixels.");
        assert_eq!(c.proposed.formula.unwrap().value.unwrap(), json!("A = N x s^2"));
    }

    #[test]
    fn nothing_is_proposed_from_sentences_that_match_no_step() {
        assert!(RulesProposer.propose(&pages("The weather was pleasant. Coffee was served.")).is_empty());
    }
}
