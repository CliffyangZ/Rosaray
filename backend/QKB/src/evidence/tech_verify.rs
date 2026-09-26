//! Technical-verification runner (kb-api §7): runs a published node's own
//! `tests/cases.yaml` through its named built-in executor. A `passed` verification
//! record is only ever *produced by* this run — it cannot be asserted by a
//! request. The seed cases' expected values come from the prototype's JavaScript,
//! so they are independent of the Rust executors they check.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::designer::runtime_adapter::{
    effective_parameters, validate_outputs, Artifact, ExecContext, ImageF32, Inputs, MaskU8, Registry,
};
use crate::kb::bundle::model::parse_yaml;
use crate::kb::bundle::read::Bundle;

pub const CASES_FILE: &str = "tests/cases.yaml";
pub const RUNNER_VERSION: &str = "1";
const IMAGE_TOLERANCE: f64 = 1e-3;

#[derive(Debug, Deserialize)]
struct CaseFile {
    #[serde(default)]
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    #[serde(default)]
    context: Option<Value>,
    #[serde(default)]
    inputs: BTreeMap<String, Value>,
    #[serde(default)]
    parameters: Option<Value>,
    #[serde(default)]
    expect: Option<Expect>,
    #[serde(default)]
    expect_error: Option<ExpectError>,
}

#[derive(Debug, Deserialize)]
struct Expect {
    outputs: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct ExpectError {
    code: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CaseResult {
    pub name: String,
    pub passed: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TestReport {
    /// `b3:` hash of the cases file, recorded as the suite identity.
    pub tests_content_id: String,
    pub cases: Vec<CaseResult>,
    pub all_passed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum TestError {
    #[error("the bundle has no tests/cases.yaml, so it cannot be verified")]
    NoTests,
    #[error("the node has no implementation this installation can run")]
    ImplementationUnavailable,
    #[error("the test cases could not be read: {0}")]
    Unreadable(String),
}

fn to_artifact(v: &Value) -> Result<Artifact, String> {
    let kind = v.get("kind").and_then(Value::as_str).ok_or("an artifact has no kind")?;
    let dim = |k: &str| v.get(k).and_then(Value::as_u64).map(|n| n as usize).ok_or(format!("an artifact has no {k}"));
    let nums = |k: &str| -> Result<Vec<f64>, String> {
        v.get(k).and_then(Value::as_array).ok_or(format!("an artifact has no {k}"))?.iter().map(|x| x.as_f64().ok_or("non-numeric pixel".to_string())).collect()
    };
    match kind {
        "image2d" => {
            let (w, h) = (dim("width")?, dim("height")?);
            let data: Vec<f32> = nums("data")?.into_iter().map(|x| x as f32).collect();
            if data.len() != w * h {
                return Err("image data does not match its dimensions".into());
            }
            Ok(Artifact::Image(ImageF32 { w, h, data }))
        }
        "mask2d" => {
            let (w, h) = (dim("width")?, dim("height")?);
            let data: Vec<u8> = nums("data")?.into_iter().map(|x| x as u8).collect();
            if data.len() != w * h {
                return Err("mask data does not match its dimensions".into());
            }
            Ok(Artifact::Mask(MaskU8 { w, h, data }))
        }
        "table" => {
            let rows = v.get("rows").and_then(Value::as_object).ok_or("a table has no rows")?;
            Ok(Artifact::Table(rows.iter().filter_map(|(k, x)| x.as_f64().map(|f| (k.clone(), f))).collect()))
        }
        other => Err(format!("unknown artifact kind {other}")),
    }
}

fn close(a: &Artifact, b: &Artifact) -> Result<(), String> {
    match (a, b) {
        (Artifact::Image(x), Artifact::Image(y)) => {
            if (x.w, x.h) != (y.w, y.h) {
                return Err("image size differs".into());
            }
            for (i, (p, q)) in x.data.iter().zip(&y.data).enumerate() {
                if f64::from((p - q).abs()) > IMAGE_TOLERANCE {
                    return Err(format!("pixel {i} is {p}, expected {q}"));
                }
            }
            Ok(())
        }
        (Artifact::Mask(x), Artifact::Mask(y)) => {
            if x == y {
                Ok(())
            } else {
                Err("mask differs from the expected mask".into())
            }
        }
        (Artifact::Table(x), Artifact::Table(y)) => {
            if x.keys().ne(y.keys()) {
                return Err("table columns differ".into());
            }
            for (k, p) in x {
                let q = y[k];
                if (p - q).abs() > 1e-6 * q.abs().max(1.0) {
                    return Err(format!("{k} is {p}, expected {q}"));
                }
            }
            Ok(())
        }
        _ => Err("output kind differs from the expected kind".into()),
    }
}

fn run_case(bundle: &Bundle, registry: &Registry, case: &Case) -> Result<(), String> {
    let contract = bundle.contract.as_ref().ok_or("the bundle has no contract")?;
    let imp = bundle.implementation.as_ref().ok_or("the bundle has no implementation")?;
    let exec = registry.resolve(&imp.implementation_id).ok_or("the implementation is not available")?;

    let mut ctx = ExecContext::default();
    if let Some(c) = &case.context {
        if let Some(src) = c.get("source") {
            if let Artifact::Image(i) = to_artifact(src)? {
                ctx.source = Some(Arc::new(i));
            }
        }
        if let Some(sp) = c.get("pixel_spacing_mm").and_then(Value::as_array) {
            if let (Some(x), Some(y)) = (sp.first().and_then(Value::as_f64), sp.get(1).and_then(Value::as_f64)) {
                ctx.pixel_spacing_mm = Some((x, y));
            }
        }
    }
    let mut inputs = Inputs::new();
    for (port, v) in &case.inputs {
        inputs.insert(port.clone(), Arc::new(to_artifact(v)?));
    }
    let params: Map<String, Value> = effective_parameters(contract, case.parameters.as_ref());
    let result = exec.execute(&ctx, &inputs, &params);

    match (&case.expect, &case.expect_error, result) {
        (_, Some(want), Err(e)) => {
            if e.code == want.code {
                Ok(())
            } else {
                Err(format!("failed with {} but {} was expected", e.code, want.code))
            }
        }
        (_, Some(want), Ok(_)) => Err(format!("succeeded but a {} failure was expected", want.code)),
        (Some(expect), None, Ok(outputs)) => {
            validate_outputs(contract, &outputs).map_err(|e| e.to_string())?;
            for (port, want) in &expect.outputs {
                let got = outputs.get(port).ok_or(format!("no output \"{port}\""))?;
                close(got, &to_artifact(want)?).map_err(|e| format!("output \"{port}\": {e}"))?;
            }
            Ok(())
        }
        (Some(_), None, Err(e)) => Err(format!("failed unexpectedly: {e}")),
        (None, None, _) => Err("the case states neither an expected output nor an expected error".into()),
    }
}

/// Runs every case in the published bundle's `tests/cases.yaml`.
pub fn run_bundle_tests(bundle: &Bundle) -> Result<TestReport, TestError> {
    let imp = bundle.implementation.as_ref().ok_or(TestError::ImplementationUnavailable)?;
    let registry = Registry::builtin();
    if registry.resolve(&imp.implementation_id).is_none() {
        return Err(TestError::ImplementationUnavailable);
    }
    let bytes = std::fs::read(bundle.dir.join(CASES_FILE)).map_err(|_| TestError::NoTests)?;
    let text = String::from_utf8(bytes.clone()).map_err(|e| TestError::Unreadable(e.to_string()))?;
    let file: CaseFile = parse_yaml(&text).map_err(TestError::Unreadable)?;
    if file.cases.is_empty() {
        return Err(TestError::NoTests);
    }
    let cases: Vec<CaseResult> = file
        .cases
        .iter()
        .map(|c| {
            let r = run_case(bundle, &registry, c);
            CaseResult { name: c.name.clone(), passed: r.is_ok(), detail: r.err() }
        })
        .collect();
    let all_passed = cases.iter().all(|c| c.passed);
    Ok(TestReport { tests_content_id: format!("b3:{}", blake3::hash(&bytes).to_hex()), cases, all_passed })
}

/// The `suite` block recorded with a passed technical verification.
pub fn suite_of(report: &TestReport) -> Value {
    json!({ "tests_content_id": report.tests_content_id, "runner_version": RUNNER_VERSION, "environment": std::env::consts::OS })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;
    use crate::kb::bundle::read::read_bundle;

    #[test]
    fn every_seed_bundle_passes_its_own_javascript_derived_cases() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("kb");
        crate::kb::ensure_layout(&root).unwrap();
        let key = MasterKey::derive("pw", &crate::crypto::generate_salt()).unwrap();
        let conn = crate::data_repository::sqlite::open(&dir.path().join("t.sqlite3"), &key).unwrap();
        crate::kb::seed::install(&conn, &root).unwrap();
        for seed in crate::kb::seed::seed_bundles() {
            let (bundle, _) = read_bundle(&root.join("nodes").join(seed.id).join(seed.version));
            let report = run_bundle_tests(&bundle.unwrap()).unwrap_or_else(|e| panic!("{}: {e}", seed.id));
            let failed: Vec<_> = report.cases.iter().filter(|c| !c.passed).collect();
            assert!(failed.is_empty(), "{}: {failed:?}", seed.id);
            assert!(report.cases.len() >= 3, "{}: needs normal, boundary and failure cases", seed.id);
        }
    }
}
