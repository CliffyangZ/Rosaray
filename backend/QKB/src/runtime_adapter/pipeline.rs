//! Running one node of an AlgoPipe graph. Shared by Preview (which caches
//! results) and official Runs (which never do): both execute a node exactly the
//! same way, so a Preview and a Run of the same pipe compute the same thing.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::{effective_parameters, validate_outputs, ExecContext, Inputs, NodeError, Outputs, Registry};
use crate::bundle::model::{Contract, GraphFile};
use crate::identity::split_endpoint;

/// Executes `instance` of `graph`: gathers its inputs from `results` (the
/// already-computed upstream outputs), runs its built-in executor, and checks
/// the outputs against the declared contract.
#[allow(clippy::too_many_arguments)]
pub fn execute_node(
    graph: &GraphFile,
    instance: &str,
    contract: &Contract,
    implementation_id: &str,
    given_parameters: Option<&Value>,
    results: &HashMap<String, Arc<Outputs>>,
    ctx: &ExecContext,
    registry: &Registry,
) -> Result<Outputs, NodeError> {
    let exec = registry
        .resolve(implementation_id)
        .ok_or_else(|| NodeError::new("implementation_unavailable", format!("{implementation_id} is not available")))?;
    let mut inputs = Inputs::new();
    for e in &graph.edges {
        let (to, to_port) = split_endpoint(&e.to);
        if to != instance {
            continue;
        }
        let (from, from_port) = split_endpoint(&e.from);
        let up = results.get(&from).ok_or_else(|| NodeError::new("input_missing", "an upstream result is missing"))?;
        let art = from_port
            .as_deref()
            .and_then(|p| up.get(p))
            .ok_or_else(|| NodeError::new("input_missing", "an upstream output is missing"))?;
        inputs.insert(to_port.unwrap_or_default(), Arc::new(art.clone()));
    }
    let params = effective_parameters(contract, given_parameters);
    let outputs = exec.execute(ctx, &inputs, &params)?;
    validate_outputs(contract, &outputs)?;
    Ok(outputs)
}
