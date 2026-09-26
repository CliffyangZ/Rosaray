//! `validate_graph`: is this working graph a valid, typed, unit- and
//! calibration-aware pipeline? Stateless — it validates the *unsaved* graph the
//! editor sends, which is what lets undo/redo stay client-side while validation
//! stays server-authoritative (research §8). Every finding carries a subject,
//! an explanation and an action (FR-005).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::Value;

use super::compat::check_ports;
use super::contract::value_is_allowed;
use super::domain_rules::rules_for;
use super::finding::{BundleRef, Finding, Severity, Subject, SubjectType};
use crate::bundle::model::{Contract, GraphFile, NodeInstance, Parameter, Port, TargetDataProfile};
use crate::identity::split_endpoint;

/// What the validator needs to know about a referenced node version.
#[derive(Debug, Clone)]
pub struct Definition {
    pub contract: Contract,
    pub deprecated: bool,
    /// Has a resolvable, trusted implementation (Preview can run it).
    pub implemented: bool,
}

pub trait DefinitionSource {
    fn definition(&self, id: &str, version: &str) -> Option<Definition>;
}

#[derive(Debug, Clone, Default)]
pub struct GraphReport {
    pub valid: bool,
    pub executable_for_preview: bool,
    pub findings: Vec<Finding>,
    /// Nodes whose earlier results are now out of date (edited nodes and
    /// everything downstream), when a `previous` graph was supplied.
    pub stale_nodes: Vec<String>,
}

pub struct GraphInput<'a> {
    pub bundle: BundleRef,
    pub graph: &'a GraphFile,
    pub previous: Option<&'a GraphFile>,
    pub profile: Option<&'a TargetDataProfile>,
    pub domain: Option<&'a str>,
}

struct Resolved {
    def: Definition,
}

fn err(bundle: &BundleRef, code: &str, subject: Subject, explanation: String, action: &str) -> Finding {
    Finding::build(Severity::Error, code, bundle, subject, explanation, action)
}

fn port<'a>(ports: &'a [Port], id: &str) -> Option<&'a Port> {
    ports.iter().find(|p| p.port_id == id)
}

pub fn validate_graph(input: &GraphInput, defs: &dyn DefinitionSource) -> GraphReport {
    let bundle = &input.bundle;
    let graph = input.graph;
    let mut findings = Vec::new();
    let mut cache: HashMap<(String, String), Option<Definition>> = HashMap::new();
    let mut resolved: BTreeMap<&str, Resolved> = BTreeMap::new();

    for node in &graph.nodes {
        let subject = Subject::new(SubjectType::NodeInstance, node.instance_id.clone());
        let key = (node.node_ref.id.clone(), node.node_ref.version.clone());
        let def = cache
            .entry(key)
            .or_insert_with(|| defs.definition(&node.node_ref.id, &node.node_ref.version))
            .clone();
        match def {
            None => findings.push(err(
                bundle,
                "dependency_unresolved",
                subject,
                format!("{}@{} is not in the knowledge base, so node \"{}\" cannot be checked.", node.node_ref.id, node.node_ref.version, node.instance_id),
                "Import or create that version, or pick a node that exists.",
            )),
            Some(def) => {
                if def.deprecated {
                    findings.push(Finding::build(
                        Severity::Warning,
                        "dependency_deprecated",
                        bundle,
                        subject,
                        format!("{}@{} is deprecated; the reference is kept as written.", node.node_ref.id, node.node_ref.version),
                        "Consider moving to the replacement version when it suits your study.",
                    ));
                }
                resolved.insert(node.instance_id.as_str(), Resolved { def });
            }
        }
    }

    // Edges: endpoints exist and run output → input.
    let mut edge_ok: Vec<Option<(&Port, &Port)>> = Vec::new();
    for e in &graph.edges {
        let subject = Subject::new(SubjectType::Edge, format!("{}->{}", e.from, e.to));
        let (from_inst, from_port) = split_endpoint(&e.from);
        let (to_inst, to_port) = split_endpoint(&e.to);
        let bad = |what: String| {
            err(
                bundle,
                "edge_endpoint_unknown",
                subject.clone(),
                what,
                "Connect an output port of one node to an input port of another, using ports the contracts declare.",
            )
        };
        let (Some(fp), Some(tp)) = (from_port, to_port) else {
            findings.push(bad(format!("The connection {} → {} does not name both ports (expected node.port).", e.from, e.to)));
            edge_ok.push(None);
            continue;
        };
        let from_def = resolved.get(from_inst.as_str());
        let to_def = resolved.get(to_inst.as_str());
        let exists = |inst: &str| graph.nodes.iter().any(|n| n.instance_id == inst);
        if !exists(&from_inst) || !exists(&to_inst) {
            findings.push(bad(format!("The connection {} → {} points at a node that is not in the pipe.", e.from, e.to)));
            edge_ok.push(None);
            continue;
        }
        // Unresolved definitions were already reported; skip port checks.
        let (Some(fd), Some(td)) = (from_def, to_def) else {
            edge_ok.push(None);
            continue;
        };
        let out = port(&fd.def.contract.outputs, &fp);
        let inp = port(&td.def.contract.inputs, &tp);
        match (out, inp) {
            (Some(o), Some(i)) => edge_ok.push(Some((o, i))),
            (o, i) => {
                if o.is_none() {
                    findings.push(bad(format!("{from_inst} has no output port \"{fp}\".")));
                }
                if i.is_none() {
                    findings.push(bad(format!("{to_inst} has no input port \"{tp}\".")));
                }
                edge_ok.push(None);
            }
        }
    }

    // Cycles.
    let cyc = find_cycle(graph);
    if let Some(nodes) = cyc {
        findings.push(err(
            bundle,
            "graph_cycle",
            Subject::new(SubjectType::NodeInstance, nodes[0].clone()),
            format!("The pipe loops back on itself through {}.", nodes.join(" → ")),
            "Remove one of the connections in the loop so data only flows forward.",
        ));
    }

    // Port compatibility per edge.
    for (e, pair) in graph.edges.iter().zip(&edge_ok) {
        if let Some((o, i)) = pair {
            for p in check_ports(o, i, &e.from, &e.to) {
                findings.push(err(
                    bundle,
                    &format!("port_incompatible.{}", p.facet),
                    Subject::new(SubjectType::Edge, format!("{}->{}", e.from, e.to)),
                    p.explanation,
                    &p.action,
                ));
            }
        }
    }
    findings.extend(source_findings(bundle, graph, &resolved, &edge_ok, cyc_free(&findings)));

    // Inputs: required connected, single-value inputs not over-occupied.
    for node in &graph.nodes {
        let Some(r) = resolved.get(node.instance_id.as_str()) else { continue };
        for inp in &r.def.contract.inputs {
            let inbound = graph
                .edges
                .iter()
                .filter(|e| {
                    let (inst, p) = split_endpoint(&e.to);
                    inst == node.instance_id && p.as_deref() == Some(inp.port_id.as_str())
                })
                .count() as u32;
            let subject = Subject::new(SubjectType::Port, format!("{}.{}", node.instance_id, inp.port_id));
            let min = inp.multiplicity.as_ref().map(|m| m.min).unwrap_or(1);
            let required = inp.required.unwrap_or(min >= 1);
            if required && inbound == 0 {
                findings.push(err(
                    bundle,
                    "input_missing",
                    subject.clone(),
                    format!("The required input \"{}\" of {} is not connected.", inp.port_id, node.instance_id),
                    "Connect an output that produces this input.",
                ));
            }
            // A port with no declared multiplicity holds one value.
            let max = inp.multiplicity.as_ref().map(|m| m.max).unwrap_or(Some(1));
            if max.is_some_and(|m| inbound > m) {
                findings.push(err(
                    bundle,
                    "input_over_occupied",
                    subject,
                    format!("The input \"{}\" of {} takes at most {} connection(s) but has {inbound}.", inp.port_id, node.instance_id, max.unwrap_or(1)),
                    "Remove the extra connections so the input has a single source.",
                ));
            }
        }
    }

    // Parameters.
    for node in &graph.nodes {
        if let Some(r) = resolved.get(node.instance_id.as_str()) {
            check_parameters(bundle, node, &r.def.contract.parameters, &mut findings);
        }
    }

    // Prerequisites against the target data profile.
    for node in &graph.nodes {
        let Some(r) = resolved.get(node.instance_id.as_str()) else { continue };
        for pre in &r.def.contract.prerequisites {
            if !prerequisite_declared(pre, input.profile) {
                let severity = if input.profile.is_some() { Severity::Error } else { Severity::Warning };
                findings.push(Finding::build(
                    severity,
                    "prerequisite_unmet",
                    bundle,
                    Subject::new(SubjectType::NodeInstance, node.instance_id.clone()),
                    format!("{} requires {} but the pipe's target data profile does not guarantee it.", node.instance_id, pre.predicate),
                    &format!("Add a `{}` requirement to the target data profile, or remove the node.", pre.predicate),
                ));
            }
        }
    }

    // Domain rules — the core never branches on the domain itself.
    for rule in rules_for(input.domain) {
        findings.extend(rule.check(bundle, graph));
    }

    let valid = !findings.iter().any(|f| f.is_error());
    let all_implemented = graph.nodes.iter().all(|n| resolved.get(n.instance_id.as_str()).is_some_and(|r| r.def.implemented));
    let stale_nodes = input.previous.map(|prev| stale_cone(prev, graph)).unwrap_or_default();
    GraphReport { valid, executable_for_preview: valid && all_implemented && !graph.nodes.is_empty(), findings, stale_nodes }
}

fn cyc_free(findings: &[Finding]) -> bool {
    !findings.iter().any(|f| f.code == "graph_cycle")
}

/// One cycle's node ids in walk order, if the instance graph has any.
fn find_cycle(graph: &GraphFile) -> Option<Vec<String>> {
    let mut next: HashMap<String, Vec<String>> = HashMap::new();
    for e in &graph.edges {
        next.entry(split_endpoint(&e.from).0).or_default().push(split_endpoint(&e.to).0);
    }
    #[derive(Clone, Copy, PartialEq)]
    enum M {
        Visiting,
        Done,
    }
    fn dfs(n: &str, next: &HashMap<String, Vec<String>>, marks: &mut HashMap<String, M>, stack: &mut Vec<String>) -> Option<Vec<String>> {
        marks.insert(n.to_string(), M::Visiting);
        stack.push(n.to_string());
        for m in next.get(n).into_iter().flatten() {
            match marks.get(m.as_str()) {
                Some(M::Visiting) => {
                    let start = stack.iter().position(|s| s == m).unwrap_or(0);
                    let mut cycle: Vec<String> = stack[start..].to_vec();
                    cycle.push(m.clone());
                    return Some(cycle);
                }
                Some(M::Done) => {}
                None => {
                    if let Some(c) = dfs(m, next, marks, stack) {
                        return Some(c);
                    }
                }
            }
        }
        stack.pop();
        marks.insert(n.to_string(), M::Done);
        None
    }
    let mut marks = HashMap::new();
    let mut ids: Vec<&str> = graph.nodes.iter().map(|n| n.instance_id.as_str()).collect();
    ids.sort();
    for id in ids {
        if !marks.contains_key(id) {
            if let Some(c) = dfs(id, &next, &mut marks, &mut Vec::new()) {
                return Some(c);
            }
        }
    }
    None
}

/// `port_incompatible.source`: inputs that must share a source (declared with
/// `same_source_as`) do not trace back to a common origin.
fn source_findings(
    bundle: &BundleRef,
    graph: &GraphFile,
    resolved: &BTreeMap<&str, Resolved>,
    edge_ok: &[Option<(&Port, &Port)>],
    acyclic: bool,
) -> Vec<Finding> {
    if !acyclic {
        return Vec::new();
    }
    let mut out = Vec::new();
    let inbound = |inst: &str, port: &str| -> Vec<String> {
        graph
            .edges
            .iter()
            .zip(edge_ok)
            .filter(|(e, ok)| {
                ok.is_some() && {
                    let (i, p) = split_endpoint(&e.to);
                    i == inst && p.as_deref() == Some(port)
                }
            })
            .map(|(e, _)| e.from.clone())
            .collect()
    };
    fn sources(
        endpoint: &str,
        graph: &GraphFile,
        resolved: &BTreeMap<&str, Resolved>,
        inbound: &dyn Fn(&str, &str) -> Vec<String>,
        depth: usize,
    ) -> BTreeSet<String> {
        let (inst, port) = split_endpoint(endpoint);
        let mut set = BTreeSet::new();
        let Some(r) = resolved.get(inst.as_str()) else {
            set.insert(inst);
            return set;
        };
        let out = port.as_deref().and_then(|p| super::graph::port(&r.def.contract.outputs, p));
        let parents: Vec<&str> = out.map(|o| o.same_source_as.iter().map(String::as_str).collect()).unwrap_or_default();
        if parents.is_empty() || depth > graph.nodes.len() + 1 {
            set.insert(inst);
            return set;
        }
        for input in parents {
            for up in inbound(&inst, input) {
                set.extend(sources(&up, graph, resolved, inbound, depth + 1));
            }
        }
        if set.is_empty() {
            set.insert(inst);
        }
        set
    }
    for node in &graph.nodes {
        let Some(r) = resolved.get(node.instance_id.as_str()) else { continue };
        for inp in &r.def.contract.inputs {
            for other in &inp.same_source_as {
                let a: BTreeSet<String> = inbound(&node.instance_id, &inp.port_id)
                    .iter()
                    .flat_map(|e| sources(e, graph, resolved, &inbound, 0))
                    .collect();
                let b: BTreeSet<String> = inbound(&node.instance_id, other)
                    .iter()
                    .flat_map(|e| sources(e, graph, resolved, &inbound, 0))
                    .collect();
                if !a.is_empty() && !b.is_empty() && a.is_disjoint(&b) {
                    out.push(err(
                        bundle,
                        "port_incompatible.source",
                        Subject::new(SubjectType::Port, format!("{}.{}", node.instance_id, inp.port_id)),
                        format!("The inputs \"{}\" and \"{other}\" of {} must come from the same source image but do not.", inp.port_id, node.instance_id),
                        "Feed both inputs from data derived from the same Image source.",
                    ));
                }
            }
        }
    }
    out
}

fn prerequisite_declared(pre: &crate::bundle::model::Predicate, profile: Option<&TargetDataProfile>) -> bool {
    let Some(profile) = profile else { return false };
    profile.require.iter().any(|req| {
        req.predicate == pre.predicate
            && match (pre.args.get("equals"), req.args.get("equals")) {
                (Some(want), Some(have)) => want == have,
                (Some(_), None) => false,
                _ => true,
            }
    })
}

fn check_parameters(bundle: &BundleRef, node: &NodeInstance, defs: &[Parameter], out: &mut Vec<Finding>) {
    let empty = serde_json::Map::new();
    let given = node.parameters.as_ref().and_then(Value::as_object).unwrap_or(&empty);
    let subject = |p: &str| Subject::new(SubjectType::Parameter, format!("{}.{p}", node.instance_id));
    let mut bad = |p: &str, explanation: String, action: &str| {
        out.push(err(bundle, "parameter_invalid", subject(p), explanation, action));
    };
    for key in given.keys() {
        if !defs.iter().any(|d| &d.parameter_id == key) {
            bad(key, format!("{} has no parameter \"{key}\".", node.instance_id), "Remove it, or use a parameter the node declares.");
        }
    }
    let effective = |id: &str| -> Option<&Value> { given.get(id).or_else(|| defs.iter().find(|d| d.parameter_id == id).and_then(|d| d.default.as_ref())) };
    for d in defs {
        match given.get(&d.parameter_id) {
            Some(v) => {
                if !type_matches(&d.param_type, v) {
                    bad(&d.parameter_id, format!("\"{}\" of {} should be a {} but is {}.", d.parameter_id, node.instance_id, d.param_type, v), "Enter a value of the declared type.");
                } else if !value_is_allowed(d, v) {
                    bad(&d.parameter_id, format!("The value {v} for \"{}\" of {} is outside what the node allows.", d.parameter_id, node.instance_id), "Choose a value inside the allowed values or range.");
                }
            }
            None => {
                let required_by_condition = d.required_if.as_ref().is_some_and(|cond| {
                    cond.iter().all(|(k, want)| effective(k).is_some_and(|have| have == want))
                });
                if (d.required || required_by_condition) && d.default.is_none() {
                    bad(&d.parameter_id, format!("The required parameter \"{}\" of {} has no value.", d.parameter_id, node.instance_id), "Set a value for it.");
                }
            }
        }
    }
}

fn type_matches(ty: &str, v: &Value) -> bool {
    match ty {
        "number" => v.is_number(),
        "integer" => v.as_i64().is_some() || v.as_f64().is_some_and(|f| f.fract() == 0.0),
        "boolean" => v.is_boolean(),
        "string" | "enum" => v.is_string(),
        _ => true,
    }
}

/// Edited nodes (new, or changed reference/parameters/inbound connections) and
/// every node downstream of them.
pub fn stale_cone(prev: &GraphFile, cur: &GraphFile) -> Vec<String> {
    let sig = |n: &NodeInstance| {
        format!("{}@{}#{}|{}", n.node_ref.id, n.node_ref.version, n.node_ref.content_id.clone().unwrap_or_default(), n.parameters.clone().unwrap_or(Value::Null))
    };
    let prev_nodes: HashMap<&str, String> = prev.nodes.iter().map(|n| (n.instance_id.as_str(), sig(n))).collect();
    let inbound = |g: &GraphFile, inst: &str| -> BTreeSet<String> {
        g.edges.iter().filter(|e| split_endpoint(&e.to).0 == inst).map(|e| format!("{}->{}", e.from, e.to)).collect()
    };
    let mut changed: HashSet<String> = HashSet::new();
    for n in &cur.nodes {
        let same = prev_nodes.get(n.instance_id.as_str()).is_some_and(|s| *s == sig(n)) && inbound(prev, &n.instance_id) == inbound(cur, &n.instance_id);
        if !same {
            changed.insert(n.instance_id.clone());
        }
    }
    let mut stale = changed.clone();
    let mut frontier: Vec<String> = changed.into_iter().collect();
    while let Some(n) = frontier.pop() {
        for e in &cur.edges {
            if split_endpoint(&e.from).0 == n {
                let to = split_endpoint(&e.to).0;
                if stale.insert(to.clone()) {
                    frontier.push(to);
                }
            }
        }
    }
    let mut out: Vec<String> = stale.into_iter().collect();
    out.sort();
    out
}
