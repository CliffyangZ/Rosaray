//! Runtime Adapter (research §10, FR-045): the *only* way a node ever runs. A
//! node's `implementation_id` is looked up in a compile-time registry of
//! trusted built-in executors; anything else is simply not resolvable —
//! imported or hand-written bundles can declare an implementation but can
//! never bring code. Preview and official Runs share these executors (AlgoNode
//! Spec: same implementation for both), and every result is checked against the
//! node's declared output contract before it becomes an artifact.

pub mod builtin;
pub mod pipeline;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::bundle::model::Contract;

#[derive(Debug, Clone, PartialEq)]
pub struct ImageF32 {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaskU8 {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

/// A value flowing along an edge.
#[derive(Debug, Clone, PartialEq)]
pub enum Artifact {
    Image(ImageF32),
    Mask(MaskU8),
    /// Named scalar measurements (e.g. pixels, mm2, components).
    Table(BTreeMap<String, f64>),
}

impl Artifact {
    /// The contract `artifact_kind` this value satisfies.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Artifact::Image(_) => "image2d",
            Artifact::Mask(_) => "mask2d",
            Artifact::Table(_) => "table",
        }
    }

    pub fn as_image(&self) -> Result<&ImageF32, NodeError> {
        match self {
            Artifact::Image(i) => Ok(i),
            other => Err(NodeError::new("wrong_input_kind", format!("expected an image but received {}", other.kind_str()))),
        }
    }

    pub fn as_mask(&self) -> Result<&MaskU8, NodeError> {
        match self {
            Artifact::Mask(m) => Ok(m),
            other => Err(NodeError::new("wrong_input_kind", format!("expected a mask but received {}", other.kind_str()))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeError {
    pub code: String,
    pub message: String,
}

impl NodeError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.to_string(), message: message.into() }
    }
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

pub type Inputs = BTreeMap<String, Arc<Artifact>>;
pub type Outputs = BTreeMap<String, Artifact>;

/// Everything about the *image under analysis* a node may need.
#[derive(Debug, Clone, Default)]
pub struct ExecContext {
    /// The Image Asset's grayscale research representation.
    pub source: Option<Arc<ImageF32>>,
    pub pixel_spacing_mm: Option<(f64, f64)>,
    pub seed: Option<u64>,
}

pub trait NodeExecutor: Send + Sync {
    fn implementation_id(&self) -> &'static str;
    /// Runs the node. `params` are the *effective* parameters (the pipe's values
    /// over the definition's author-supplied defaults); nothing is defaulted here.
    fn execute(&self, ctx: &ExecContext, inputs: &Inputs, params: &Map<String, Value>) -> Result<Outputs, NodeError>;
}

/// Compile-time registry of trusted built-in executors.
pub struct Registry {
    executors: HashMap<&'static str, Box<dyn NodeExecutor>>,
}

impl Registry {
    pub fn builtin() -> Self {
        let mut executors: HashMap<&'static str, Box<dyn NodeExecutor>> = HashMap::new();
        for e in builtin::all() {
            executors.insert(e.implementation_id(), e);
        }
        Self { executors }
    }

    /// Resolves only built-in ids (FR-045); everything else is `None`.
    pub fn resolve(&self, implementation_id: &str) -> Option<&dyn NodeExecutor> {
        self.executors.get(implementation_id).map(|b| b.as_ref())
    }

    pub fn ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<_> = self.executors.keys().copied().collect();
        ids.sort();
        ids
    }
}

/// Checks a node's outputs against its declared output contract. An invalid
/// output is a node failure and produces no artifact.
pub fn validate_outputs(contract: &Contract, outputs: &Outputs) -> Result<(), NodeError> {
    for decl in &contract.outputs {
        let got = outputs.get(&decl.port_id).ok_or_else(|| {
            NodeError::new("invalid_output", format!("the node produced no value for its declared output \"{}\"", decl.port_id))
        })?;
        if let Some(kind) = &decl.artifact_kind {
            if kind != got.kind_str() {
                return Err(NodeError::new(
                    "invalid_output",
                    format!("output \"{}\" should be {kind} but the node produced {}", decl.port_id, got.kind_str()),
                ));
            }
        }
        match got {
            Artifact::Mask(m) if m.data.iter().any(|v| *v > 1) => {
                return Err(NodeError::new("invalid_output", format!("output \"{}\" is a mask with values outside 0 and 1", decl.port_id)))
            }
            Artifact::Image(i) if i.data.iter().any(|v| !v.is_finite()) => {
                return Err(NodeError::new("invalid_output", format!("output \"{}\" contains non-finite values", decl.port_id)))
            }
            Artifact::Table(t) if t.values().any(|v| !v.is_finite()) => {
                return Err(NodeError::new("invalid_output", format!("output \"{}\" contains non-finite values", decl.port_id)))
            }
            _ => {}
        }
    }
    if let Some(extra) = outputs.keys().find(|k| !contract.outputs.iter().any(|d| &d.port_id == *k)) {
        return Err(NodeError::new("invalid_output", format!("the node produced an undeclared output \"{extra}\"")));
    }
    Ok(())
}

/// The parameters a node actually runs with: the pipe's values over the
/// definition's *author-supplied* defaults. Nothing else is ever filled in.
pub fn effective_parameters(contract: &Contract, given: Option<&Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for p in &contract.parameters {
        if let Some(d) = &p.default {
            out.insert(p.parameter_id.clone(), d.clone());
        }
    }
    if let Some(Value::Object(g)) = given {
        for (k, v) in g {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

pub fn require_f64(params: &Map<String, Value>, key: &str) -> Result<f64, NodeError> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| NodeError::new("parameter_missing", format!("the numeric parameter \"{key}\" has no value")))
}

pub fn require_str<'a>(params: &'a Map<String, Value>, key: &str) -> Result<&'a str, NodeError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| NodeError::new("parameter_missing", format!("the parameter \"{key}\" has no value")))
}

pub fn input<'a>(inputs: &'a Inputs, port: &str) -> Result<&'a Artifact, NodeError> {
    inputs
        .get(port)
        .map(|a| a.as_ref())
        .ok_or_else(|| NodeError::new("input_missing", format!("the input \"{port}\" was not supplied")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_resolves_exactly_the_ids_the_catalog_treats_as_implemented() {
        let registry = Registry::builtin();
        let mut catalog: Vec<&str> = crate::catalog::status::BUILTIN_IMPLEMENTATIONS.to_vec();
        catalog.sort();
        assert_eq!(registry.ids(), catalog);
        assert!(registry.resolve("builtin.onnx").is_none());
        assert!(registry.resolve("evil.shell").is_none());
    }

    #[test]
    fn outputs_are_checked_against_the_declared_contract() {
        let contract: Contract = crate::bundle::model::parse_yaml(
            "outputs:\n  - { port_id: mask, artifact_kind: mask2d, unit: none }\n",
        )
        .unwrap();
        let ok: Outputs = [("mask".to_string(), Artifact::Mask(MaskU8 { w: 1, h: 1, data: vec![1] }))].into();
        assert!(validate_outputs(&contract, &ok).is_ok());
        let wrong_kind: Outputs = [("mask".to_string(), Artifact::Table(BTreeMap::new()))].into();
        assert_eq!(validate_outputs(&contract, &wrong_kind).unwrap_err().code, "invalid_output");
        let bad_values: Outputs = [("mask".to_string(), Artifact::Mask(MaskU8 { w: 1, h: 1, data: vec![7] }))].into();
        assert!(validate_outputs(&contract, &bad_values).is_err());
        assert!(validate_outputs(&contract, &Outputs::new()).is_err());
        let extra: Outputs = [
            ("mask".to_string(), Artifact::Mask(MaskU8 { w: 1, h: 1, data: vec![1] })),
            ("bonus".to_string(), Artifact::Table(BTreeMap::new())),
        ]
        .into();
        assert!(validate_outputs(&contract, &extra).is_err());
    }
}
