//! Inspector assembly (US3, FR-010/FR-011): everything a researcher needs to
//! decide whether to trust a node, with maturity, technical verification,
//! dataset validation, availability and deprecation kept as *separate* fields
//! and the three evidence types kept apart. Information that is not known is
//! the literal `"unknown"`, never omitted.

use std::path::Path;

use rusqlite::Connection;
use serde_json::{json, Value};

use crate::domain::dataset::ImageAsset;
use crate::kb::bundle::model::{Kind, Port};
use crate::kb::bundle::read::read_bundle;
use crate::kb::bundle::service::KbServiceError;
use crate::kb::catalog::query::{findings_for_path, path_of};
use crate::kb::catalog::status::{derive, verification_subject};
use crate::kb::evidence::chain::read_chain;
use crate::kb::evidence::deprecation;
use crate::kb::evidence::record::group;
use crate::kb::evidence::verification::chain_dir as verification_dir;
use crate::kb::profile::{unmet_prerequisites, ImageFacts};

const UNKNOWN: &str = "unknown";

fn text(v: &Option<String>) -> Value {
    match v.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => json!(s),
        _ => json!(UNKNOWN),
    }
}

fn port_json(p: &Port) -> Value {
    let mut v = serde_json::to_value(p).expect("port serializes");
    for key in ["artifact_kind", "dtype", "unit", "coordinate_space", "calibration"] {
        if v.get(key).is_none_or(Value::is_null) {
            v[key] = json!(UNKNOWN);
        }
    }
    v
}

pub fn inspect(
    conn: &Connection,
    root: &Path,
    kind: Kind,
    id: &str,
    version: &str,
    image: Option<&ImageAsset>,
) -> Result<Value, KbServiceError> {
    let path = path_of(conn, kind.as_str(), id, version, "published")?.ok_or(KbServiceError::NotFound)?;
    let (bundle, read_findings) = read_bundle(&root.join(&path));
    let bundle = bundle.ok_or(KbServiceError::NotFound)?;
    let modified = read_findings.iter().any(|f| f.code == "published_bundle_modified");
    let derived = derive(root, &bundle, modified);
    let h = &bundle.header;

    let amendments = deprecation::read(root, id, version)?;
    let amendment_list: Vec<Value> = amendments
        .records
        .iter()
        .map(|r| json!({ "seq": r.seq, "kind": r.kind, "reason": r.reason, "at": r.created_at, "author": r.author, "detail": r.extra }))
        .collect();

    let mut out = json!({
        "kind": kind.as_str(),
        "id": id,
        "version": version,
        "name": text(&h.name),
        "summary": text(&h.summary),
        "domain": text(&h.domain),
        "intended_use": text(&h.intended_use),
        "limitations": text(&h.limitations),
        "research_use_only": true,
        "content_id": bundle.lock.as_ref().map(|l| l.content_id.clone()),
        "amendments": amendment_list,
        "findings": findings_for_path(conn, &path)?,
    });

    if let Some(c) = &bundle.contract {
        let repro = c.reproducibility.as_ref();
        out["purpose"] = text(&c.purpose);
        out["method"] = text(&c.method);
        out["output_interpretation"] = text(&c.output_interpretation);
        out["population_or_data_scope"] = text(&c.population_or_data_scope);
        out["ports"] = json!({
            "inputs": c.inputs.iter().map(port_json).collect::<Vec<_>>(),
            "outputs": c.outputs.iter().map(port_json).collect::<Vec<_>>(),
        });
        out["parameters"] = json!(c.parameters);
        out["prerequisites"] = json!(c.prerequisites);
        out["reproducibility"] = json!({
            "deterministic": repro.map(|r| json!(r.deterministic)).unwrap_or(json!(UNKNOWN)),
            "cacheable": repro.map(|r| json!(r.cacheable)).unwrap_or(json!(UNKNOWN)),
            "needs_seed": repro.map(|r| json!(r.needs_seed)).unwrap_or(json!(UNKNOWN)),
            "external_state": repro.map(|r| json!(r.external_state)).unwrap_or(json!(UNKNOWN)),
        });
    }

    if kind == Kind::Algonode {
        let groups = group(&bundle.evidence);
        out["evidence"] = json!({
            "method_source": groups.method_source,
            "technical_verification": groups.technical_verification,
            "dataset_validation": groups.dataset_validation,
        });
        let chain = read_chain(&verification_dir(root, id))?;
        let history: Vec<Value> = chain
            .records
            .iter()
            .filter(|r| r.subject.get("node_id").and_then(Value::as_str) == Some(id))
            .map(|r| json!({ "seq": r.seq, "type": r.extra.get("type"), "event": r.kind, "reason": r.reason, "at": r.created_at, "subject": r.subject }))
            .collect();
        let (impl_status, imp) = match &bundle.implementation {
            None => ("none", Value::Null),
            Some(i) => (
                if derived.trust == crate::kb::bundle::model::Trust::Untrusted {
                    "untrusted"
                } else if !derived.implementation_resolvable {
                    "unavailable"
                } else {
                    "available"
                },
                json!({ "implementation_id": i.implementation_id, "implementation_version": i.implementation_version, "declared_trust": i.trust }),
            ),
        };
        out["implementation"] = json!({ "status": impl_status, "declared": imp, "effective_trust": derived.trust, "resolvable": derived.implementation_resolvable });
        out["verification_history"] = json!(history);
        out["verification_subject"] = json!(verification_subject(&bundle));
    } else {
        let deps: Vec<Value> = bundle
            .graph
            .iter()
            .flat_map(|g| g.nodes.iter())
            .map(|n| {
                let row: Option<(String, Option<String>)> = conn
                    .query_row(
                        "SELECT availability, maturity FROM kb_catalog_entry WHERE id = ?1 AND version = ?2 AND status = 'published' LIMIT 1",
                        rusqlite::params![n.node_ref.id, n.node_ref.version],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .ok();
                json!({
                    "instance_id": n.instance_id,
                    "ref": format!("{}@{}", n.node_ref.id, n.node_ref.version),
                    "content_id": n.node_ref.content_id,
                    "resolved": row.is_some(),
                    "availability": row.as_ref().map(|r| r.0.clone()).unwrap_or_else(|| "unavailable".into()),
                    "maturity": row.and_then(|r| r.1),
                })
            })
            .collect();
        out["dependencies"] = json!(deps);
        out["release_kind"] = json!(bundle.lock.as_ref().and_then(|l| l.release_kind.clone()).unwrap_or_else(|| "knowledge".into()));
        out["target_data_profile"] = json!(bundle.graph.as_ref().and_then(|g| g.target_data_profile.clone()));
    }

    // Five separate dimensions — there is deliberately no single "verified".
    out["status"] = json!({
        "maturity": derived.maturity.unwrap_or("not_applicable"),
        "technical_verification": derived.verification.as_str(),
        "dataset_validation": derived.dataset_validation,
        "availability": derived.availability,
        "deprecation": derived.deprecation,
    });

    if let (Some(image), Some(c)) = (image, &bundle.contract) {
        let facts = ImageFacts::from_asset(image);
        let unmet = unmet_prerequisites(c.prerequisites.iter().map(|p| (p.predicate.as_str(), &p.args)), &facts);
        out["compatibility"] = json!({
            "image_asset_id": image.id,
            "compatible": unmet.is_empty(),
            "unmet": unmet.iter().map(|u| json!({
                "code": "prerequisite_unmet",
                "predicate": u.predicate,
                "expected": u.expected,
                "observed": u.observed,
                "explanation": format!("This node requires {} ({}), but the selected image has {}.", u.predicate, u.expected, u.observed),
                "action": "Choose an image that satisfies it, or supply the missing metadata for this image.",
            })).collect::<Vec<_>>(),
        });
    }
    Ok(out)
}
