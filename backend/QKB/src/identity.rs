//! Stable identity rules for Knowledge Bundles (research §3): ID and SemVer
//! validation, canonical JSON, content identity and computational identity.
//! Nothing here derives from a path or a display name (FR-037, FR-041).

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::contract::finding::{BundleRef, Finding, Severity, Subject};
use crate::graph_identity::{compute_graph_identity, GraphEdge, GraphNode, PipelineGraph};
use crate::bundle::model::GraphFile;

const ID_PATTERN: &str = r"^[a-z][a-z0-9]*(\.[a-z0-9-]+)+$";

fn id_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(ID_PATTERN).expect("static id regex is valid"))
}

pub fn is_valid_id(id: &str) -> bool {
    id_regex().is_match(id)
}

pub fn is_valid_version(version: &str) -> bool {
    semver::Version::parse(version).is_ok()
}

/// `id_invalid` / `version_invalid` findings for a bundle's declared identity.
pub fn validate_identity(id: &str, version: &str) -> Vec<Finding> {
    let bundle = BundleRef::new(id, Some(version.to_string()));
    let mut findings = Vec::new();
    if !is_valid_id(id) {
        findings.push(Finding::build(
            Severity::Error,
            "id_invalid",
            &bundle,
            Subject::bundle(id),
            format!("The bundle ID \"{id}\" is not a valid stable ID (expected a reverse-DNS-style slug matching {ID_PATTERN})."),
            "Rename the ID to lowercase dot-separated segments such as \"rosaray.threshold\".",
        ));
    }
    if !is_valid_version(version) {
        findings.push(Finding::build(
            Severity::Error,
            "version_invalid",
            &bundle,
            Subject::bundle(id),
            format!("The version \"{version}\" is not a valid SemVer version."),
            "Use MAJOR.MINOR.PATCH, for example \"1.0.0\".",
        ));
    }
    findings
}

/// Canonical JSON: object keys sorted, no insignificant whitespace, numbers in
/// shortest round-trip form with integral floats written as integers (so `1`
/// and `1.0` hash alike). Strings are emitted as UTF-8, escaping only what
/// JSON requires, so Unicode text is byte-stable.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out.into_bytes()
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&canonical_number(n)),
        Value::String(s) => out.push_str(&serde_json::to_string(s).expect("string serializes")),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("key serializes"));
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
    }
}

fn canonical_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    let f = n.as_f64().expect("finite JSON number");
    if f == 0.0 {
        return "0".to_string();
    }
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        return format!("{}", f as i64);
    }
    // serde_json formats f64 with the shortest round-trip representation.
    serde_json::Number::from_f64(f)
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn b3(bytes: &[u8]) -> String {
    format!("b3:{}", blake3::hash(bytes).to_hex())
}

/// Every file under `dir` as sorted `(relative '/'-path, bytes-hash)` pairs.
pub fn file_hashes(dir: &Path) -> std::io::Result<Vec<(String, String)>> {
    fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, String)>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(cur)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(base, &path, out)?;
            } else {
                let rel = path
                    .strip_prefix(base)
                    .expect("walked path is under base")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, b3(&std::fs::read(&path)?)));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// Content identity of a bundle: BLAKE3 over the sorted
/// `(relative_path, file_blake3)` list, excluding `bundle.lock` (research §3).
pub fn content_id(bundle_dir: &Path) -> std::io::Result<String> {
    let pairs = file_hashes(bundle_dir)?;
    Ok(content_id_of(pairs.iter().map(|(p, h)| (p.as_str(), h.as_str()))))
}

/// Same hash from an already-computed `(path, file_hash)` list.
pub fn content_id_of<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut sorted: Vec<(&str, &str)> = pairs.into_iter().filter(|(p, _)| *p != "bundle.lock").collect();
    sorted.sort();
    let mut hasher = blake3::Hasher::new();
    for (path, hash) in sorted {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    format!("b3:{}", hasher.finalize().to_hex())
}

/// Computational identity of a pipe (FR-007, SC-010): node instances
/// (instance id, `ref@version`, `content_id`, canonical parameters), port to
/// port edges, and executable pins. Layout, labels and descriptions never
/// enter it. It extends 001's `compute_graph_identity` so Preview and Run
/// keep agreeing on identical graphs.
pub fn computational_identity(graph: &GraphFile) -> String {
    let mut pg = PipelineGraph::default();
    for node in &graph.nodes {
        pg.nodes.push(GraphNode {
            node_id: node.instance_id.clone(),
            node_type: node.node_ref.id.clone(),
            implementation_version: node.node_ref.version.clone(),
            canonical_parameters: node.parameters.clone().unwrap_or(Value::Object(Default::default())),
            reproducible: true,
            seed: None,
        });
    }
    for edge in &graph.edges {
        let (from, from_port) = split_endpoint(&edge.from);
        let (to, to_port) = split_endpoint(&edge.to);
        pg.edges.push(GraphEdge {
            from,
            to,
            from_port,
            to_port,
        });
    }

    let mut refs: Vec<Value> = graph
        .nodes
        .iter()
        .map(|n| {
            serde_json::json!({
                "instance_id": n.instance_id,
                "ref": format!("{}@{}", n.node_ref.id, n.node_ref.version),
                "content_id": n.node_ref.content_id,
            })
        })
        .collect();
    refs.sort_by_key(|v| v["instance_id"].as_str().unwrap_or_default().to_string());

    let mut pins: Vec<Value> = graph
        .implementation_pins
        .iter()
        .map(|p| {
            let mut assets = p.asset_content_ids.clone();
            assets.sort();
            serde_json::json!({
                "instance_id": p.instance_id,
                "implementation_id": p.implementation_id,
                "version": p.version,
                "asset_content_ids": assets,
            })
        })
        .collect();
    pins.sort_by_key(|v| v["instance_id"].as_str().unwrap_or_default().to_string());

    let doc = serde_json::json!({
        "graph": compute_graph_identity(&pg),
        "nodes": refs,
        "pins": pins,
    });
    b3(&canonical_json(&doc))
}

/// `"inst.port"` → (`"inst"`, `Some("port")`); a bare `"inst"` has no port.
pub fn split_endpoint(endpoint: &str) -> (String, Option<String>) {
    match endpoint.split_once('.') {
        Some((inst, port)) => (inst.to_string(), Some(port.to_string())),
        None => (endpoint.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::model::{GraphFile, NodeInstance, NodeRef, Pin};
    use serde_json::json;

    #[test]
    fn id_and_version_rules() {
        assert!(is_valid_id("rosaray.threshold"));
        assert!(is_valid_id("acme.image-source"));
        assert!(!is_valid_id("Threshold"));
        assert!(!is_valid_id("nodots"));
        assert!(!is_valid_id("a.B"));
        assert!(is_valid_version("1.0.0"));
        assert!(!is_valid_version("1.0"));
        assert!(!is_valid_version("latest"));
        let f = validate_identity("Bad", "1.0");
        let codes: Vec<_> = f.iter().map(|f| f.code.as_str()).collect();
        assert_eq!(codes, vec!["id_invalid", "version_invalid"]);
        assert!(validate_identity("rosaray.ok", "2.1.3").is_empty());
    }

    #[test]
    fn canonical_json_sorts_keys_and_strips_whitespace() {
        let a = json!({"b": 1, "a": {"y": [1, 2], "x": null}});
        let b = json!({"a": {"x": null, "y": [1, 2]}, "b": 1});
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(String::from_utf8(canonical_json(&a)).unwrap(), r#"{"a":{"x":null,"y":[1,2]},"b":1}"#);
    }

    #[test]
    fn canonical_json_number_formatting() {
        assert_eq!(canonical_json(&json!(1.0)), b"1");
        assert_eq!(canonical_json(&json!(1)), b"1");
        assert_eq!(canonical_json(&json!(-0.0)), b"0");
        assert_eq!(canonical_json(&json!(0.1)), b"0.1");
        assert_eq!(canonical_json(&json!(2.5e-7)), b"2.5e-7");
    }

    #[test]
    fn canonical_json_unicode_is_stable() {
        let v = json!({"名": "齲蝕 ünï"});
        let bytes = canonical_json(&v);
        assert_eq!(bytes, canonical_json(&v));
        assert_eq!(String::from_utf8(bytes).unwrap(), "{\"名\":\"齲蝕 ünï\"}");
    }

    fn graph() -> GraphFile {
        GraphFile {
            schema: "quantify-kb/1".into(),
            nodes: vec![
                NodeInstance {
                    instance_id: "src".into(),
                    node_ref: NodeRef { id: "rosaray.image-source".into(), version: "1.0.0".into(), content_id: Some("b3:aa".into()) },
                    parameters: None,
                    layout: Some(json!({"x": 1, "y": 2})),
                    label: None,
                },
                NodeInstance {
                    instance_id: "th".into(),
                    node_ref: NodeRef { id: "rosaray.threshold".into(), version: "1.0.0".into(), content_id: Some("b3:bb".into()) },
                    parameters: Some(json!({"mode": "otsu"})),
                    layout: None,
                    label: None,
                },
            ],
            edges: vec![crate::bundle::model::Edge { from: "src.image".into(), to: "th.image".into() }],
            target_data_profile: None,
            implementation_pins: vec![],
        }
    }

    #[test]
    fn layout_and_label_do_not_change_identity_but_parameters_do() {
        let base = computational_identity(&graph());
        let mut moved = graph();
        moved.nodes[0].layout = Some(json!({"x": 900, "y": 900}));
        moved.nodes[1].label = Some("renamed".into());
        assert_eq!(computational_identity(&moved), base);

        let mut changed = graph();
        changed.nodes[1].parameters = Some(json!({"mode": "manual"}));
        assert_ne!(computational_identity(&changed), base);

        let mut pinned = graph();
        pinned.implementation_pins = vec![Pin {
            instance_id: "th".into(),
            implementation_id: "builtin.threshold".into(),
            version: "1".into(),
            asset_content_ids: vec![],
        }];
        assert_ne!(computational_identity(&pinned), base);

        let mut new_dep = graph();
        new_dep.nodes[1].node_ref.content_id = Some("b3:cc".into());
        assert_ne!(computational_identity(&new_dep), base);
    }

    #[test]
    fn content_id_ignores_bundle_lock_and_tracks_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "a").unwrap();
        let one = content_id(dir.path()).unwrap();
        std::fs::write(dir.path().join("bundle.lock"), "anything").unwrap();
        assert_eq!(content_id(dir.path()).unwrap(), one);
        std::fs::write(dir.path().join("a.md"), "b").unwrap();
        assert_ne!(content_id(dir.path()).unwrap(), one);
    }
}
