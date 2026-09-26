//! Port compatibility (FR-003, FR-004): can this output feed that input?
//! Missing metadata is never assumed compatible — if exactly one side omits a
//! property, the edge is reported. Two sides that both omit a property (data
//! that has no such notion, e.g. a scalar with no coordinate space) agree.

use crate::kb::bundle::model::Port;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incompatibility {
    /// `type | unit | space | calibration`
    pub facet: &'static str,
    pub explanation: String,
    pub action: String,
}

fn compare(
    facet: &'static str,
    label: &str,
    out: &Option<String>,
    inp: &Option<String>,
    out_name: &str,
    in_name: &str,
    problems: &mut Vec<Incompatibility>,
) {
    let norm = |o: &Option<String>| o.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    match (norm(out), norm(inp)) {
        (Some(a), Some(b)) if a != b => problems.push(Incompatibility {
            facet,
            explanation: format!("{out_name} produces {label} \"{a}\" but {in_name} expects \"{b}\"."),
            action: format!("Insert a node that converts {label} \"{a}\" to \"{b}\", or connect a compatible port."),
        }),
        (Some(_), None) | (None, Some(_)) => problems.push(Incompatibility {
            facet,
            explanation: format!("The {label} of {out_name} or {in_name} is not declared, so compatibility cannot be confirmed."),
            action: format!("Declare the {label} on both ports in their contracts."),
        }),
        _ => {}
    }
}

/// Every reason `out` cannot feed `inp`. Empty means compatible.
pub fn check_ports(out: &Port, inp: &Port, out_name: &str, in_name: &str) -> Vec<Incompatibility> {
    let mut problems = Vec::new();
    compare("type", "artifact kind", &out.artifact_kind, &inp.artifact_kind, out_name, in_name, &mut problems);
    compare("type", "data type", &out.dtype, &inp.dtype, out_name, in_name, &mut problems);
    compare("unit", "unit", &out.unit, &inp.unit, out_name, in_name, &mut problems);
    compare("space", "coordinate space", &out.coordinate_space, &inp.coordinate_space, out_name, in_name, &mut problems);
    compare("calibration", "calibration", &out.calibration, &inp.calibration, out_name, in_name, &mut problems);
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(kind: &str, unit: Option<&str>, space: Option<&str>, cal: Option<&str>) -> Port {
        Port {
            port_id: "p".into(),
            name: None,
            artifact_kind: Some(kind.into()),
            shape: None,
            dtype: None,
            value_domain: None,
            unit: unit.map(str::to_string),
            coordinate_space: space.map(str::to_string),
            calibration: cal.map(str::to_string),
            multiplicity: None,
            required: None,
            same_source_as: vec![],
        }
    }

    #[test]
    fn identical_ports_are_compatible() {
        let p = port("image2d", Some("intensity"), Some("image-pixel"), Some("none"));
        assert!(check_ports(&p, &p.clone(), "a.out", "b.in").is_empty());
    }

    #[test]
    fn each_facet_is_reported_separately_with_an_action() {
        let out = port("image2d", Some("mm"), Some("image-pixel"), Some("none"));
        let inp = port("mask2d", Some("pixel"), Some("world"), Some("mm"));
        let facets: Vec<_> = check_ports(&out, &inp, "a.out", "b.in").iter().map(|i| i.facet).collect();
        assert_eq!(facets, vec!["type", "unit", "space", "calibration"]);
        assert!(check_ports(&out, &inp, "a.out", "b.in").iter().all(|i| !i.action.is_empty()));
    }

    #[test]
    fn one_sided_missing_metadata_is_incompatible_but_both_missing_agrees() {
        let a = port("scalar", Some("none"), None, None);
        let b = port("scalar", Some("none"), Some("time"), None);
        assert_eq!(check_ports(&a, &b, "a", "b")[0].facet, "space");
        assert!(check_ports(&a, &a.clone(), "a", "b").is_empty());
        let no_unit = port("scalar", None, None, None);
        assert_eq!(check_ports(&no_unit, &a, "a", "b")[0].facet, "unit");
    }
}
