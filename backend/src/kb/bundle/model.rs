//! Typed serde models for the `quantify-kb/1` machine files
//! (contracts/bundle-format.md, data-model.md). Reading is tolerant of
//! unknown fields; completeness is judged by validators, not by serde, so a
//! half-written draft still loads and can be reported on.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Algonode,
    Algopipe,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Algonode => "algonode",
            Kind::Algopipe => "algopipe",
        }
    }
}

// ---- contract.yaml ----

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Multiplicity {
    #[serde(default)]
    pub min: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Port {
    pub port_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub artifact_kind: Option<String>,
    #[serde(default)]
    pub shape: Option<Value>,
    #[serde(default)]
    pub dtype: Option<String>,
    #[serde(default)]
    pub value_domain: Option<Value>,
    /// Must be declared explicitly, `none` included; absent ⇒
    /// `port_unit_undeclared` (AlgoNode Spec).
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub coordinate_space: Option<String>,
    #[serde(default)]
    pub calibration: Option<String>,
    #[serde(default)]
    pub multiplicity: Option<Multiplicity>,
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default)]
    pub same_source_as: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Range {
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub parameter_id: String,
    #[serde(rename = "type")]
    pub param_type: String,
    #[serde(default)]
    pub meaning: Option<String>,
    #[serde(default)]
    pub required: bool,
    /// Present only if the author supplied it; never invented (FR-026).
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub allowed: Option<Vec<Value>>,
    #[serde(default)]
    pub range: Option<Range>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub required_if: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub cross_constraints: Vec<Value>,
    /// Where a `default` came from (`paper`); a paper-sourced default must carry `evidence_ref`.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
}

/// A checkable predicate (same vocabulary as the Target Data Profile).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Predicate {
    pub predicate: String,
    #[serde(flatten)]
    pub args: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reproducibility {
    pub deterministic: bool,
    pub cacheable: bool,
    #[serde(default)]
    pub needs_seed: bool,
    #[serde(default)]
    pub external_state: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Contract {
    #[serde(default)]
    pub schema: String,
    /// Optional cross-check fields (`node_type` = bundle id).
    #[serde(default)]
    pub node_type: Option<String>,
    #[serde(default)]
    pub definition_version: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub output_interpretation: Option<String>,
    #[serde(default)]
    pub population_or_data_scope: Option<String>,
    #[serde(default)]
    pub inputs: Vec<Port>,
    #[serde(default)]
    pub outputs: Vec<Port>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default)]
    pub prerequisites: Vec<Predicate>,
    #[serde(default)]
    pub reproducibility: Option<Reproducibility>,
}

// ---- implementation.yaml ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    Builtin,
    Untrusted,
    Trusted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplAsset {
    pub path: String,
    pub blake3: String,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Implementation {
    pub implementation_id: String,
    pub implementation_version: String,
    pub trust: Trust,
    #[serde(default)]
    pub assets: Vec<ImplAsset>,
}

// ---- graph.yaml ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRef {
    pub id: String,
    /// A SemVer string; may be `draft` in drafts only.
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInstance {
    pub instance_id: String,
    #[serde(rename = "ref")]
    pub node_ref: NodeRef,
    /// Computation-affecting parameters only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    /// Presentation only; excluded from computational identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Value>,
    /// Presentation only; excluded from computational identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// `"instance.port"`
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfilePredicate {
    pub predicate: String,
    #[serde(flatten)]
    pub args: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetDataProfile {
    #[serde(default)]
    pub profile_version: u32,
    #[serde(default)]
    pub require: Vec<ProfilePredicate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pin {
    pub instance_id: String,
    pub implementation_id: String,
    pub version: String,
    #[serde(default)]
    pub asset_content_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphFile {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub nodes: Vec<NodeInstance>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_data_profile: Option<TargetDataProfile>,
    #[serde(default)]
    pub implementation_pins: Vec<Pin>,
}

// ---- bundle.lock ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    pub path: String,
    pub blake3: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleLock {
    pub schema: String,
    pub id: String,
    pub version: String,
    pub content_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub computational_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_kind: Option<String>,
    /// What the researcher was told and accepted at publication, e.g.
    /// `dataset_validation_missing` (FR-029). Disclosed, never a gate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disclosures: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_summary: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_summary: Option<Value>,
    pub files: Vec<LockFile>,
}

// ---- evidence record (references/*.yaml) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    MethodSource,
    TechnicalVerification,
    DatasetValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatientDataStatus {
    None,
    #[default]
    Unresolved,
    ReviewedNonPatient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliesTo {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub content_id: Option<String>,
    #[serde(default)]
    pub implementation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    #[serde(rename = "type")]
    pub evidence_type: EvidenceType,
    pub applies_to: AppliesTo,
    #[serde(default)]
    pub locator: Option<Value>,
    /// Required for `dataset_validation`.
    #[serde(default)]
    pub scope: Option<Value>,
    #[serde(default)]
    pub distributable: bool,
    #[serde(default)]
    pub patient_data_status: PatientDataStatus,
}

// ---- helpers ----

pub fn parse_yaml<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, String> {
    serde_yaml_ng::from_str(text).map_err(|e| e.to_string())
}

pub fn to_yaml<T: Serialize>(value: &T) -> Result<String, String> {
    serde_yaml_ng::to_string(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_contract_example() {
        let yaml = r#"
schema: quantify-kb/1
category: segmentation
purpose: Produce a binary mask from an intensity image.
inputs:
  - port_id: image
    name: Image
    artifact_kind: image2d
    dtype: float32
    unit: none
    coordinate_space: image-pixel
    calibration: none
    multiplicity: { min: 1, max: 1 }
    required: true
outputs:
  - port_id: mask
    artifact_kind: mask2d
    value_domain: [0, 1]
    unit: none
    same_source_as: [image]
parameters:
  - parameter_id: mode
    type: enum
    allowed: [otsu, manual]
    required: true
    default: manual
  - parameter_id: value
    type: number
    range: { min: 0, max: 255 }
    unit: intensity
    required_if: { mode: manual }
prerequisites: []
reproducibility: { deterministic: true, cacheable: true, needs_seed: false, external_state: [] }
"#;
        let c: Contract = parse_yaml(yaml).unwrap();
        assert_eq!(c.inputs[0].unit.as_deref(), Some("none"));
        assert_eq!(c.outputs[0].same_source_as, vec!["image"]);
        assert_eq!(c.parameters[1].range.as_ref().unwrap().max, Some(255.0));
        assert!(c.reproducibility.unwrap().deterministic);
    }

    #[test]
    fn parses_the_documented_graph_example() {
        let yaml = r#"
schema: quantify-kb/1
nodes:
  - instance_id: src
    ref: { id: rosaray.image-source, version: 1.0.0, content_id: "b3:9f" }
  - instance_id: th
    ref: { id: rosaray.threshold, version: 1.0.0, content_id: "b3:41" }
    parameters: { mode: otsu }
    layout: { x: 320, y: 120 }
edges:
  - { from: src.image, to: th.image }
target_data_profile:
  profile_version: 1
  require:
    - { predicate: modality, equals: intraoral-photo }
implementation_pins:
  - { instance_id: th, implementation_id: builtin.threshold, version: "1", asset_content_ids: [] }
"#;
        let g: GraphFile = parse_yaml(yaml).unwrap();
        assert_eq!(g.nodes[1].node_ref.version, "1.0.0");
        assert_eq!(g.edges[0].to, "th.image");
        let p = &g.target_data_profile.unwrap().require[0];
        assert_eq!(p.predicate, "modality");
        assert_eq!(p.args["equals"], "intraoral-photo");
        assert_eq!(g.implementation_pins[0].implementation_id, "builtin.threshold");
    }
}
