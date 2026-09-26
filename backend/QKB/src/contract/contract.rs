//! `validate_contract`: is an AlgoNode contract complete and self-consistent?
//! Never invents a value — a missing unit, default or reproducibility flag is
//! reported, not filled in (FR-004, FR-026).

use std::collections::HashSet;

use super::finding::{BundleRef, Finding, Severity, Subject, SubjectType};
use crate::kb::bundle::model::{Contract, Parameter, Port};
use crate::kb::profile::is_known_predicate;

pub fn validate_contract(bundle: &BundleRef, c: &Contract) -> Vec<Finding> {
    let mut out = Vec::new();
    check_ports(bundle, "input", &c.inputs, &mut out);
    check_ports(bundle, "output", &c.outputs, &mut out);

    let input_ids: HashSet<&str> = c.inputs.iter().map(|p| p.port_id.as_str()).collect();
    for port in &c.outputs {
        for src in &port.same_source_as {
            if !input_ids.contains(src.as_str()) {
                out.push(Finding::build(
                    Severity::Error,
                    "port_same_source_unknown",
                    bundle,
                    Subject::new(SubjectType::Port, port.port_id.clone()),
                    format!("Output \"{}\" says it derives from input \"{src}\", but no such input exists.", port.port_id),
                    "Name an existing input port in same_source_as, or remove the entry.",
                ));
            }
        }
    }

    check_parameters(bundle, &c.parameters, &mut out);

    for pre in &c.prerequisites {
        if !is_known_predicate(&pre.predicate) {
            out.push(Finding::build(
                Severity::Error,
                "unsupported_profile_predicate",
                bundle,
                Subject::new(SubjectType::Bundle, format!("prerequisites.{}", pre.predicate)),
                format!("The prerequisite \"{}\" is not in the supported predicate vocabulary, so it cannot be checked.", pre.predicate),
                "Use a supported predicate (modality, color_mode, format, min_width_px, min_height_px, pixel_spacing_present, pixel_spacing_range_mm, mask_present, coordinate_space).",
            ));
        }
    }

    match &c.reproducibility {
        None => out.push(Finding::build(
            Severity::Warning,
            "reproducibility_undeclared",
            bundle,
            Subject::file("contract.yaml"),
            "The contract does not declare determinism, cacheability or seed needs, so the node is treated as unknown.".into(),
            "Add a reproducibility block (deterministic, cacheable, needs_seed, external_state).",
        )),
        Some(r) => {
            if r.cacheable && !r.deterministic && !r.needs_seed && r.external_state.is_empty() {
                out.push(Finding::build(
                    Severity::Error,
                    "reproducibility_inconsistent",
                    bundle,
                    Subject::file("contract.yaml"),
                    "The node is marked cacheable but not deterministic, and declares neither a seed nor external state.".into(),
                    "Set needs_seed: true, list its external_state, or mark it not cacheable.",
                ));
            }
        }
    }
    out
}

fn check_ports(bundle: &BundleRef, direction: &str, ports: &[Port], out: &mut Vec<Finding>) {
    let mut seen = HashSet::new();
    for port in ports {
        let subject = Subject::new(SubjectType::Port, port.port_id.clone());
        if port.port_id.trim().is_empty() {
            out.push(Finding::build(
                Severity::Error,
                "port_id_missing",
                bundle,
                Subject::file("contract.yaml"),
                format!("An {direction} port has an empty port_id."),
                "Give every port a stable port_id.",
            ));
            continue;
        }
        if !seen.insert(port.port_id.as_str()) {
            out.push(Finding::build(
                Severity::Error,
                "port_duplicate",
                bundle,
                subject.clone(),
                format!("The {direction} port id \"{}\" is declared more than once.", port.port_id),
                "Make each port_id unique among the node's inputs (or outputs).",
            ));
        }
        if port.unit.as_deref().map(str::trim).unwrap_or("").is_empty() {
            out.push(Finding::build(
                Severity::Error,
                "port_unit_undeclared",
                bundle,
                subject.clone(),
                format!("The {direction} port \"{}\" does not declare a unit; an unknown unit cannot be left implicit.", port.port_id),
                "Declare the unit explicitly, using `none` if the value is dimensionless.",
            ));
        }
        if port.artifact_kind.as_deref().map(str::trim).unwrap_or("").is_empty() {
            out.push(Finding::build(
                Severity::Error,
                "port_artifact_kind_missing",
                bundle,
                subject.clone(),
                format!("The {direction} port \"{}\" does not declare what kind of artifact it carries.", port.port_id),
                "Set artifact_kind (for example image2d, mask2d, scalar).",
            ));
        }
        if let Some(m) = &port.multiplicity {
            if m.max.is_some_and(|max| max < m.min) {
                out.push(Finding::build(
                    Severity::Error,
                    "port_multiplicity_invalid",
                    bundle,
                    subject,
                    format!("The {direction} port \"{}\" has a multiplicity whose max is below its min.", port.port_id),
                    "Make max at least min.",
                ));
            }
        }
    }
}

fn check_parameters(bundle: &BundleRef, params: &[Parameter], out: &mut Vec<Finding>) {
    let ids: HashSet<&str> = params.iter().map(|p| p.parameter_id.as_str()).collect();
    let mut seen = HashSet::new();
    for p in params {
        let subject = Subject::new(SubjectType::Parameter, p.parameter_id.clone());
        let mut bad = |code: &str, explanation: String, action: &str| {
            out.push(Finding::build(Severity::Error, code, bundle, subject.clone(), explanation, action));
        };
        if !seen.insert(p.parameter_id.as_str()) {
            bad(
                "parameter_duplicate",
                format!("The parameter id \"{}\" is declared more than once.", p.parameter_id),
                "Make each parameter_id unique.",
            );
        }
        if p.param_type == "enum" && p.allowed.as_ref().is_none_or(|a| a.is_empty()) {
            bad(
                "parameter_definition_invalid",
                format!("The enum parameter \"{}\" lists no allowed values.", p.parameter_id),
                "Add the allowed values.",
            );
        }
        if let Some(r) = &p.range {
            if let (Some(min), Some(max)) = (r.min, r.max) {
                if min > max {
                    bad(
                        "parameter_definition_invalid",
                        format!("The parameter \"{}\" has a range whose min is above its max.", p.parameter_id),
                        "Fix the range bounds.",
                    );
                }
            }
        }
        if p.default.is_some() && p.source.as_deref() == Some("paper") && p.evidence_ref.as_deref().is_none_or(str::is_empty) {
            bad(
                "parameter_default_unsourced",
                format!("The default for \"{}\" is marked as coming from a paper but cites no evidence record.", p.parameter_id),
                "Add the evidence_ref of the method-source record that states it, or remove the default.",
            );
        }
        if let Some(default) = &p.default {
            if !value_is_allowed(p, default) {
                bad(
                    "parameter_default_invalid",
                    format!("The default given for \"{}\" is outside its own allowed values or range.", p.parameter_id),
                    "Change the default, or widen the allowed values or range.",
                );
            }
        }
        if let Some(cond) = &p.required_if {
            for key in cond.keys() {
                if !ids.contains(key.as_str()) {
                    bad(
                        "parameter_definition_invalid",
                        format!("The parameter \"{}\" is required_if on \"{key}\", which is not a declared parameter.", p.parameter_id),
                        "Reference an existing parameter_id.",
                    );
                }
            }
        }
    }
}

/// Does `value` satisfy `p`'s `allowed` list and numeric `range`?
pub fn value_is_allowed(p: &Parameter, value: &serde_json::Value) -> bool {
    if let Some(allowed) = &p.allowed {
        if !allowed.iter().any(|a| a == value) {
            return false;
        }
    }
    if let Some(range) = &p.range {
        if let Some(n) = value.as_f64() {
            if range.min.is_some_and(|min| n < min) || range.max.is_some_and(|max| n > max) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::bundle::model::parse_yaml;

    fn check(yaml: &str) -> Vec<String> {
        let c: Contract = parse_yaml(yaml).unwrap();
        validate_contract(&BundleRef::new("acme.x", None), &c)
            .into_iter()
            .map(|f| f.code)
            .collect()
    }

    const GOOD: &str = r#"
schema: quantify-kb/1
inputs:
  - { port_id: image, artifact_kind: image2d, unit: none }
outputs:
  - { port_id: mask, artifact_kind: mask2d, unit: none, same_source_as: [image] }
parameters:
  - { parameter_id: mode, type: enum, allowed: [otsu, manual], default: manual }
  - { parameter_id: value, type: number, range: { min: 0, max: 255 }, required_if: { mode: manual } }
reproducibility: { deterministic: true, cacheable: true }
"#;

    #[test]
    fn a_complete_contract_has_no_findings() {
        assert_eq!(check(GOOD), Vec::<String>::new());
    }

    #[test]
    fn missing_unit_is_reported_never_defaulted() {
        let y = GOOD.replace("{ port_id: image, artifact_kind: image2d, unit: none }", "{ port_id: image, artifact_kind: image2d }");
        assert_eq!(check(&y), vec!["port_unit_undeclared"]);
    }

    #[test]
    fn detects_structural_problems() {
        let y = r#"
schema: quantify-kb/1
inputs:
  - { port_id: a, artifact_kind: image2d, unit: none }
  - { port_id: a, artifact_kind: image2d, unit: none }
outputs:
  - { port_id: o, artifact_kind: mask2d, unit: none, same_source_as: [zzz] }
parameters:
  - { parameter_id: p, type: enum }
  - { parameter_id: q, type: number, range: { min: 5, max: 1 }, default: 9 }
prerequisites:
  - { predicate: shoe_size }
reproducibility: { deterministic: false, cacheable: true }
"#;
        let codes = check(y);
        for want in [
            "port_duplicate",
            "port_same_source_unknown",
            "parameter_definition_invalid",
            "parameter_default_invalid",
            "unsupported_profile_predicate",
            "reproducibility_inconsistent",
        ] {
            assert!(codes.iter().any(|c| c == want), "{want} missing from {codes:?}");
        }
    }

    #[test]
    fn missing_reproducibility_is_a_warning() {
        let c: Contract = parse_yaml("schema: quantify-kb/1\n").unwrap();
        let f = validate_contract(&BundleRef::new("a.b", None), &c);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Warning);
    }
}
