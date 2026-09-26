//! Target Data Profile / prerequisite vocabulary and evaluation (research §14).
//! The predicate set is closed: an unknown predicate is
//! `unsupported_profile_predicate` and blocks — it is never ignored. Facts that
//! are not known about an image are `"unknown"` and never satisfy a predicate;
//! nothing is assumed (FR-003).

use serde::Serialize;
use serde_json::{json, Map, Value};


/// The closed predicate vocabulary shared by Target Data Profiles and node
/// prerequisites.
pub const PREDICATES: &[&str] = &[
    "modality",
    "color_mode",
    "format",
    "min_width_px",
    "min_height_px",
    "pixel_spacing_present",
    "pixel_spacing_range_mm",
    "mask_present",
    "coordinate_space",
];

pub fn is_known_predicate(name: &str) -> bool {
    PREDICATES.contains(&name)
}

/// What is declared about the data a method would be applied to.
#[derive(Debug, Clone, Default)]
pub struct ImageFacts {
    pub format: Option<String>,
    pub width: u32,
    pub height: u32,
    pub pixel_spacing_mm: Option<(f64, f64)>,
    pub mask_present: bool,
    pub color_mode: Option<String>,
    pub modality: Option<String>,
    pub coordinate_space: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Unmet {
    pub predicate: String,
    pub expected: Value,
    /// The observed value, or the literal `"unknown"`.
    pub observed: Value,
}

const UNKNOWN: &str = "unknown";

fn or_unknown<T: Into<Value>>(v: Option<T>) -> Value {
    v.map(Into::into).unwrap_or_else(|| json!(UNKNOWN))
}

/// Checks one predicate against `facts`. `Err` carries what was expected and
/// what was observed.
pub fn evaluate_predicate(name: &str, args: &Map<String, Value>, facts: &ImageFacts) -> Result<(), Unmet> {
    let unmet = |expected: Value, observed: Value| Unmet { predicate: name.to_string(), expected, observed };
    let equals = args.get("equals");
    let eq_str = |have: &Option<String>| -> Result<(), Unmet> {
        let want = equals.cloned().unwrap_or(Value::Null);
        match have {
            Some(h) if Some(&json!(h)) == equals => Ok(()),
            other => Err(unmet(want, or_unknown(other.clone()))),
        }
    };
    let want_bool = equals.and_then(Value::as_bool).unwrap_or(true);
    match name {
        "modality" => eq_str(&facts.modality),
        "color_mode" => eq_str(&facts.color_mode),
        "format" => eq_str(&facts.format),
        "coordinate_space" => eq_str(&facts.coordinate_space),
        "min_width_px" | "min_height_px" => {
            let min = args.get("value").and_then(Value::as_f64).unwrap_or(0.0);
            let have = if name == "min_width_px" { facts.width } else { facts.height };
            if have as f64 >= min {
                Ok(())
            } else {
                Err(unmet(json!(min), json!(have)))
            }
        }
        "pixel_spacing_present" => {
            let present = facts.pixel_spacing_mm.is_some();
            if present == want_bool {
                Ok(())
            } else {
                Err(unmet(json!(want_bool), json!(present)))
            }
        }
        "mask_present" => {
            if facts.mask_present == want_bool {
                Ok(())
            } else {
                Err(unmet(json!(want_bool), json!(facts.mask_present)))
            }
        }
        "pixel_spacing_range_mm" => {
            let expected = json!({ "min": args.get("min"), "max": args.get("max") });
            match facts.pixel_spacing_mm {
                None => Err(unmet(expected, json!(UNKNOWN))),
                Some((x, y)) => {
                    let min = args.get("min").and_then(Value::as_f64);
                    let max = args.get("max").and_then(Value::as_f64);
                    let ok = |v: f64| min.is_none_or(|m| v >= m) && max.is_none_or(|m| v <= m);
                    if ok(x) && ok(y) {
                        Ok(())
                    } else {
                        Err(unmet(expected, json!({ "x": x, "y": y })))
                    }
                }
            }
        }
        // Closed vocabulary: anything else can never be satisfied.
        _ => Err(unmet(json!("a supported predicate"), json!("unsupported"))),
    }
}

/// Every prerequisite in `preds` that `facts` does not satisfy.
pub fn unmet_prerequisites<'a>(
    preds: impl IntoIterator<Item = (&'a str, &'a Map<String, Value>)>,
    facts: &ImageFacts,
) -> Vec<Unmet> {
    preds
        .into_iter()
        .filter_map(|(name, args)| evaluate_predicate(name, args, facts).err())
        .collect()
}

/// One predicate a profile requires that the dataset does not satisfy.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Unsatisfied {
    pub predicate: String,
    pub expected: Value,
    /// What was observed on the first image that failed, or `"unknown"` / `"unsupported"`.
    pub observed: Value,
    /// How many of the dataset's images fail it.
    pub failing_images: usize,
    pub total_images: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProfileEvaluation {
    pub satisfied: bool,
    pub unsatisfied: Vec<Unsatisfied>,
}

/// Evaluates a Target Data Profile against a Dataset Version's images: every
/// image must satisfy every predicate. An unsupported predicate is never
/// ignored — it is reported as unsatisfied (`unsupported`), which blocks.
pub fn evaluate_profile<'a>(
    predicates: impl IntoIterator<Item = (&'a str, &'a Map<String, Value>)>,
    images: &[ImageFacts],
) -> ProfileEvaluation {
    let mut unsatisfied = Vec::new();
    for (name, args) in predicates {
        let mut first: Option<Unmet> = None;
        let mut failing = 0;
        if !is_known_predicate(name) {
            unsatisfied.push(Unsatisfied {
                predicate: name.to_string(),
                expected: json!("a supported predicate"),
                observed: json!("unsupported"),
                failing_images: images.len().max(1),
                total_images: images.len(),
            });
            continue;
        }
        for facts in images {
            if let Err(u) = evaluate_predicate(name, args, facts) {
                failing += 1;
                first.get_or_insert(u);
            }
        }
        if let Some(u) = first {
            unsatisfied.push(Unsatisfied { predicate: name.to_string(), expected: u.expected, observed: u.observed, failing_images: failing, total_images: images.len() });
        }
    }
    ProfileEvaluation { satisfied: unsatisfied.is_empty(), unsatisfied }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    fn facts() -> ImageFacts {
        ImageFacts {
            format: Some("png".into()),
            width: 800,
            height: 600,
            pixel_spacing_mm: None,
            mask_present: false,
            color_mode: None,
            modality: Some("intraoral-photo".into()),
            coordinate_space: Some("image-pixel".into()),
        }
    }

    #[test]
    fn missing_spacing_fails_pixel_spacing_present_naming_the_predicate() {
        let e = evaluate_predicate("pixel_spacing_present", &args(json!({ "equals": true })), &facts()).unwrap_err();
        assert_eq!(e.predicate, "pixel_spacing_present");
        assert_eq!(e.observed, json!(false));
        let mut with = facts();
        with.pixel_spacing_mm = Some((0.05, 0.05));
        assert!(evaluate_predicate("pixel_spacing_present", &args(json!({ "equals": true })), &with).is_ok());
        assert!(evaluate_predicate("pixel_spacing_range_mm", &args(json!({ "min": 0.01, "max": 0.1 })), &with).is_ok());
        assert!(evaluate_predicate("pixel_spacing_range_mm", &args(json!({ "min": 0.1 })), &with).is_err());
    }

    #[test]
    fn unknown_facts_are_reported_as_unknown_never_assumed() {
        let e = evaluate_predicate("color_mode", &args(json!({ "equals": "rgb" })), &facts()).unwrap_err();
        assert_eq!(e.observed, json!("unknown"));
        assert!(evaluate_predicate("modality", &args(json!({ "equals": "intraoral-photo" })), &facts()).is_ok());
    }

    #[test]
    fn size_format_and_mask_predicates() {
        assert!(evaluate_predicate("min_width_px", &args(json!({ "value": 512 })), &facts()).is_ok());
        assert!(evaluate_predicate("min_height_px", &args(json!({ "value": 1024 })), &facts()).is_err());
        assert!(evaluate_predicate("format", &args(json!({ "equals": "png" })), &facts()).is_ok());
        assert!(evaluate_predicate("mask_present", &args(json!({ "equals": true })), &facts()).is_err());
    }

    #[test]
    fn a_profile_must_hold_for_every_image_and_names_the_failing_predicate() {
        let mut calibrated = facts();
        calibrated.pixel_spacing_mm = Some((0.05, 0.05));
        let need = args(json!({ "equals": true }));
        let none = evaluate_profile([("pixel_spacing_present", &need)], &[calibrated.clone(), calibrated.clone()]);
        assert!(none.satisfied);
        let some = evaluate_profile([("pixel_spacing_present", &need)], &[calibrated, facts()]);
        assert!(!some.satisfied);
        assert_eq!(some.unsatisfied[0].predicate, "pixel_spacing_present");
        assert_eq!((some.unsatisfied[0].failing_images, some.unsatisfied[0].total_images), (1, 2));
        let unsupported = evaluate_profile([("shoe_size", &args(json!({})))], &[facts()]);
        assert_eq!(unsupported.unsatisfied[0].observed, json!("unsupported"));
    }

    #[test]
    fn unsupported_predicates_never_pass() {
        let e = evaluate_predicate("shoe_size", &args(json!({})), &facts()).unwrap_err();
        assert_eq!(e.observed, json!("unsupported"));
    }
}
