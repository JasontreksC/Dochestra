//! Pre-run validation and execution planning (CONTEXT §4.1.2, RFP 1.6).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};

use super::validator::{validate_params, validate_structure};
use crate::catalog::spec;
use crate::types::{NodeKind, ValidationIssue, WorkflowGraph};

/// Where one connected input socket gets its data from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBinding {
    pub socket_id: String,
    pub source_node: String,
    pub source_socket: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedNode {
    pub id: String,
    pub kind: NodeKind,
    pub params: serde_json::Value,
    /// Connected input sockets only, in catalog order.
    pub inputs: Vec<InputBinding>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionPlan {
    /// Nodes to execute, in a deterministic topological order.
    pub nodes: Vec<PlannedNode>,
    /// Ids of the `isOutput` nodes, in graph order.
    pub outputs: Vec<String>,
}

impl ExecutionPlan {
    /// Planned nodes that consume an output of `id` (deduplicated, plan order).
    pub fn dependents(&self, id: &str) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|n| n.inputs.iter().any(|b| b.source_node == id))
            .map(|n| n.id.as_str())
            .collect()
    }

    /// Distinct source nodes feeding `id` (input order). Empty for unknown ids.
    pub fn upstream(&self, id: &str) -> Vec<&str> {
        let mut seen = HashSet::new();
        self.nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| {
                n.inputs
                    .iter()
                    .map(|b| b.source_node.as_str())
                    .filter(|s| seen.insert(*s))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Full pre-run validation; on success returns the execution plan.
///
/// Stages: structure → (cycles, output node, unconnected inputs, params). Structural issues
/// short-circuit; otherwise all remaining issues are reported together in the order of
/// CONTEXT §4.1.2. No plan is produced if any issue exists (including cycles).
pub fn plan_run(graph: &WorkflowGraph) -> Result<ExecutionPlan, Vec<ValidationIssue>> {
    let structural = validate_structure(graph);
    if !structural.is_empty() {
        return Err(structural);
    }

    let index_of: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    // Edges as (source index, target index); structure validation guarantees both resolve.
    let edges: Vec<(usize, usize)> = graph
        .edges
        .iter()
        .map(|e| (index_of[e.source.as_str()], index_of[e.target.as_str()]))
        .collect();

    let mut issues = Vec::new();

    let cycles = find_cycles(graph.nodes.len(), &edges);
    for members in cycles {
        issues.push(ValidationIssue::CycleDetected {
            node_ids: members.iter().map(|&i| graph.nodes[i].id.clone()).collect(),
        });
    }

    let outputs: Vec<usize> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.is_output)
        .map(|(i, _)| i)
        .collect();
    if outputs.is_empty() {
        issues.push(ValidationIssue::NoOutputNode);
    }

    let target = reachable_upstream(graph.nodes.len(), &edges, &outputs);

    // Connected input sockets per (target node index, socket id) -> graph edge index.
    let mut incoming: HashMap<(usize, &str), usize> = HashMap::new();
    for (ei, edge) in graph.edges.iter().enumerate() {
        incoming.insert((edges[ei].1, edge.target_handle.as_str()), ei);
    }

    let mut planned: HashMap<usize, PlannedNode> = HashMap::new();
    for (ni, node) in graph.nodes.iter().enumerate() {
        if !target.contains(&ni) {
            continue;
        }
        let mut bindings = Vec::new();
        for socket in &spec(node.kind).inputs {
            match incoming.get(&(ni, socket.id.as_str())) {
                Some(&ei) => bindings.push(InputBinding {
                    socket_id: socket.id.clone(),
                    source_node: graph.edges[ei].source.clone(),
                    source_socket: graph.edges[ei].source_handle.clone(),
                }),
                None if socket.required => issues.push(ValidationIssue::UnconnectedInput {
                    node_id: node.id.clone(),
                    socket_id: socket.id.clone(),
                }),
                None => {}
            }
        }
        if let Some(message) = validate_params(node) {
            issues.push(ValidationIssue::InvalidParams {
                node_id: node.id.clone(),
                message,
            });
        }
        planned.insert(
            ni,
            PlannedNode {
                id: node.id.clone(),
                kind: node.kind,
                params: node.params.clone(),
                inputs: bindings,
            },
        );
    }

    if !issues.is_empty() {
        return Err(issues);
    }

    let order = topological_order(graph.nodes.len(), &edges, &target);
    Ok(ExecutionPlan {
        nodes: order
            .into_iter()
            .filter_map(|i| planned.remove(&i))
            .collect(),
        outputs: outputs.iter().map(|&i| graph.nodes[i].id.clone()).collect(),
    })
}

/// Cycle member sets (SCCs of size > 1 and self-loops); each sorted by graph order, and the
/// sets ordered by their first member.
fn find_cycles(node_count: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut g: DiGraph<(), ()> = DiGraph::with_capacity(node_count, edges.len());
    let idx: Vec<NodeIndex> = (0..node_count).map(|_| g.add_node(())).collect();
    for &(s, t) in edges {
        g.add_edge(idx[s], idx[t], ());
    }
    let self_loops: HashSet<usize> = edges.iter().filter(|(s, t)| s == t).map(|e| e.0).collect();

    let mut cycles: Vec<Vec<usize>> = tarjan_scc(&g)
        .into_iter()
        .filter_map(|scc| {
            let mut members: Vec<usize> = scc.into_iter().map(NodeIndex::index).collect();
            let is_cycle = members.len() > 1 || self_loops.contains(&members[0]);
            members.sort_unstable();
            is_cycle.then_some(members)
        })
        .collect();
    cycles.sort_unstable();
    cycles
}

/// Nodes reachable from `outputs` by following edges backwards (outputs included).
fn reachable_upstream(
    node_count: usize,
    edges: &[(usize, usize)],
    outputs: &[usize],
) -> HashSet<usize> {
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    for &(s, t) in edges {
        preds[t].push(s);
    }
    let mut seen: HashSet<usize> = outputs.iter().copied().collect();
    let mut queue: VecDeque<usize> = outputs.iter().copied().collect();
    while let Some(n) = queue.pop_front() {
        for &p in &preds[n] {
            if seen.insert(p) {
                queue.push_back(p);
            }
        }
    }
    seen
}

/// Kahn's algorithm over the `target` subgraph; ties broken by graph (array) order.
fn topological_order(
    node_count: usize,
    edges: &[(usize, usize)],
    target: &HashSet<usize>,
) -> Vec<usize> {
    let mut in_degree = vec![0usize; node_count];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    let mut seen_pairs: HashSet<(usize, usize)> = HashSet::new();
    for &(s, t) in edges {
        if target.contains(&s) && target.contains(&t) && seen_pairs.insert((s, t)) {
            in_degree[t] += 1;
            succs[s].push(t);
        }
    }
    let mut ready: BinaryHeap<Reverse<usize>> = target
        .iter()
        .copied()
        .filter(|&n| in_degree[n] == 0)
        .map(Reverse)
        .collect();
    let mut order = Vec::with_capacity(target.len());
    while let Some(Reverse(n)) = ready.pop() {
        order.push(n);
        for &m in &succs[n] {
            in_degree[m] -= 1;
            if in_degree[m] == 0 {
                ready.push(Reverse(m));
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::{GraphEdge, GraphNode, Position};

    const BUNDLE_ID: &str = "3f2b8a52-6b0c-4f0e-9d53-0a1f0c1d2e3f";

    fn node(id: &str, kind: NodeKind, is_output: bool) -> GraphNode {
        let params = match kind {
            NodeKind::Bundle => json!({"bundleId": BUNDLE_ID}),
            other => spec(other).default_params.clone(),
        };
        GraphNode {
            id: id.to_owned(),
            kind,
            position: Position { x: 0.0, y: 0.0 },
            params,
            is_output,
        }
    }

    fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> GraphEdge {
        GraphEdge {
            id: id.to_owned(),
            source: s.to_owned(),
            source_handle: sh.to_owned(),
            target: t.to_owned(),
            target_handle: th.to_owned(),
        }
    }

    fn graph(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> WorkflowGraph {
        WorkflowGraph { nodes, edges }
    }

    fn ids(plan: &ExecutionPlan) -> Vec<&str> {
        plan.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    fn pipeline() -> WorkflowGraph {
        graph(
            vec![
                node("b", NodeKind::Bundle, false),
                node("p", NodeKind::PickPdf, false),
                node("x", NodeKind::Extract, true),
            ],
            vec![
                edge("e1", "b", "files", "p", "files"),
                edge("e2", "p", "pdf", "x", "pdf"),
            ],
        )
    }

    fn invalid_params(g: &WorkflowGraph) -> Vec<(String, String)> {
        plan_run(g)
            .unwrap_err()
            .into_iter()
            .filter_map(|i| match i {
                ValidationIssue::InvalidParams { node_id, message } => Some((node_id, message)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_simple_pipeline_is_planned_in_order() {
        let plan = plan_run(&pipeline()).unwrap();
        assert_eq!(ids(&plan), ["b", "p", "x"]);
        assert_eq!(plan.outputs, ["x"]);
        assert_eq!(
            plan.nodes[2].inputs,
            vec![InputBinding {
                socket_id: "pdf".into(),
                source_node: "p".into(),
                source_socket: "pdf".into()
            }]
        );
        assert!(plan.nodes[0].inputs.is_empty());
        assert_eq!(plan.nodes[0].params, json!({"bundleId": BUNDLE_ID}));
        assert_eq!(plan.upstream("x"), ["p"]);
        assert_eq!(plan.dependents("b"), ["p"]);
        assert!(plan.dependents("x").is_empty());
        assert!(plan.upstream("nope").is_empty());
    }

    #[test]
    fn b_nodes_not_reaching_an_output_are_ignored() {
        let mut g = pipeline();
        // Unconnected translate (required inputs missing) that never feeds an output.
        g.nodes.push(node("t", NodeKind::Translate, false));
        let plan = plan_run(&g).unwrap();
        assert_eq!(ids(&plan), ["b", "p", "x"]);
    }

    #[test]
    fn c_cycle_reports_both_members() {
        let g = graph(
            vec![
                node("t1", NodeKind::Translate, true),
                node("t2", NodeKind::Translate, false),
            ],
            vec![
                edge("e1", "t1", "md", "t2", "md"),
                edge("e2", "t2", "md", "t1", "md"),
            ],
        );
        let issues = plan_run(&g).unwrap_err();
        assert_eq!(
            issues[0],
            ValidationIssue::CycleDetected {
                node_ids: vec!["t1".into(), "t2".into()]
            }
        );
        // Other issues are reported alongside the cycle (the unconnected `settings` inputs).
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, ValidationIssue::UnconnectedInput { .. }))
        );
    }

    #[test]
    fn d_self_loop_is_a_cycle() {
        let g = graph(
            vec![node("t", NodeKind::Translate, true)],
            vec![edge("e1", "t", "md", "t", "md")],
        );
        let issues = plan_run(&g).unwrap_err();
        assert_eq!(
            issues[0],
            ValidationIssue::CycleDetected {
                node_ids: vec!["t".into()]
            }
        );
    }

    #[test]
    fn e_no_output_node() {
        let mut g = pipeline();
        g.nodes[2].is_output = false;
        assert_eq!(
            plan_run(&g).unwrap_err(),
            vec![ValidationIssue::NoOutputNode]
        );
    }

    #[test]
    fn f_unconnected_required_input_on_target_node() {
        let g = graph(vec![node("x", NodeKind::Extract, true)], vec![]);
        assert_eq!(
            plan_run(&g).unwrap_err(),
            vec![ValidationIssue::UnconnectedInput {
                node_id: "x".into(),
                socket_id: "pdf".into()
            }]
        );
    }

    #[test]
    fn g_optional_summary_images_may_stay_unconnected() {
        let g = graph(
            vec![
                node("b", NodeKind::Bundle, false),
                node("p", NodeKind::PickPdf, false),
                node("x", NodeKind::Extract, false),
                node("s", NodeKind::Summary, true),
            ],
            vec![
                edge("e1", "b", "files", "p", "files"),
                edge("e2", "p", "pdf", "x", "pdf"),
                edge("e3", "x", "md", "s", "md"),
            ],
        );
        let plan = plan_run(&g).unwrap();
        assert_eq!(ids(&plan), ["b", "p", "x", "s"]);
        let summary = plan.nodes.last().unwrap();
        assert_eq!(summary.inputs.len(), 1);
        assert_eq!(summary.inputs[0].socket_id, "md");
    }

    #[test]
    fn h_param_rules() {
        let check = |mutate: &dyn Fn(&mut WorkflowGraph)| {
            let mut g = pipeline();
            mutate(&mut g);
            invalid_params(&g)
        };

        // bundleId missing / null / non-UUID / non-string
        for params in [
            json!({}),
            json!({"bundleId": null}),
            json!({"bundleId": "abc"}),
            json!({"bundleId": 5}),
        ] {
            let got = check(&|g| g.nodes[0].params = params.clone());
            assert_eq!(got.len(), 1, "{params}");
            assert_eq!(got[0].0, "b");
        }
        // pick index: -1, "a", 1.5, missing
        for params in [
            json!({"index": -1}),
            json!({"index": "a"}),
            json!({"index": 1.5}),
            json!({}),
        ] {
            let got = check(&|g| g.nodes[1].params = params.clone());
            assert_eq!(got.len(), 1, "{params}");
            assert_eq!(got[0].0, "p");
        }
        // valid index values
        for params in [json!({"index": 0}), json!({"index": 7})] {
            let mut g = pipeline();
            g.nodes[1].params = params;
            assert!(plan_run(&g).is_ok());
        }
        // params must be an object
        let got = check(&|g| g.nodes[2].params = json!([]));
        assert_eq!(
            got,
            vec![("x".to_owned(), "params 는 JSON 객체여야 합니다".to_owned())]
        );

        // json_input value missing / null / present
        let ji = |params: serde_json::Value| {
            let mut n = node("j", NodeKind::JsonInput, true);
            n.params = params;
            graph(vec![n], vec![])
        };
        assert_eq!(invalid_params(&ji(json!({}))).len(), 1);
        assert_eq!(invalid_params(&ji(json!({"value": null}))).len(), 1);
        assert!(plan_run(&ji(json!({"value": {}}))).is_ok());
        assert!(plan_run(&ji(json!({"value": 0}))).is_ok());
    }

    #[test]
    fn i_diamond_fan_out_runs_shared_node_once() {
        let g = graph(
            vec![
                node("b", NodeKind::Bundle, false),
                node("p", NodeKind::PickPdf, false),
                node("x", NodeKind::Extract, false),
                node("s", NodeKind::Summary, true),
                node("r", NodeKind::Rag, true),
            ],
            vec![
                edge("e1", "b", "files", "p", "files"),
                edge("e2", "p", "pdf", "x", "pdf"),
                edge("e3", "x", "md", "s", "md"),
                edge("e4", "x", "md", "r", "md"),
            ],
        );
        let plan = plan_run(&g).unwrap();
        assert_eq!(ids(&plan), ["b", "p", "x", "s", "r"]);
        assert_eq!(plan.nodes.iter().filter(|n| n.id == "x").count(), 1);
        assert_eq!(plan.dependents("x"), ["s", "r"]);
        assert_eq!(plan.outputs, ["s", "r"]);
    }

    #[test]
    fn j_structural_issues_short_circuit() {
        let g = graph(
            vec![
                node("b", NodeKind::Bundle, false),
                node("x", NodeKind::Extract, false),
            ],
            vec![edge("e1", "b", "files", "x", "pdf")],
        );
        // Also has NO_OUTPUT_NODE, which must not be reported.
        let issues = plan_run(&g).unwrap_err();
        assert_eq!(issues.len(), 1);
        assert!(matches!(issues[0], ValidationIssue::TypeMismatch { .. }));
    }

    #[test]
    fn k_planning_is_deterministic_and_ties_follow_graph_order() {
        let g = graph(
            vec![
                node("p2", NodeKind::PickPdf, false),
                node("b", NodeKind::Bundle, false),
                node("p1", NodeKind::PickPdf, false),
                node("x1", NodeKind::Extract, true),
                node("x2", NodeKind::Extract, true),
            ],
            vec![
                edge("e1", "b", "files", "p1", "files"),
                edge("e2", "b", "files", "p2", "files"),
                edge("e3", "p1", "pdf", "x1", "pdf"),
                edge("e4", "p2", "pdf", "x2", "pdf"),
            ],
        );
        let first = plan_run(&g).unwrap();
        assert_eq!(first, plan_run(&g).unwrap());
        // b first (only source); then p2 before p1 (graph order), x1/x2 as they become ready.
        assert_eq!(ids(&first), ["b", "p2", "p1", "x1", "x2"]);
    }

    #[test]
    fn empty_graph_has_no_output_node() {
        assert_eq!(
            plan_run(&WorkflowGraph::default()).unwrap_err(),
            vec![ValidationIssue::NoOutputNode]
        );
    }

    #[test]
    fn issue_order_is_cycle_output_then_per_node() {
        // Cycle among non-output nodes, no output node: CYCLE_DETECTED then NO_OUTPUT_NODE.
        let g = graph(
            vec![
                node("t1", NodeKind::Translate, false),
                node("t2", NodeKind::Translate, false),
            ],
            vec![
                edge("e1", "t1", "md", "t2", "md"),
                edge("e2", "t2", "md", "t1", "md"),
            ],
        );
        let issues = plan_run(&g).unwrap_err();
        assert!(matches!(issues[0], ValidationIssue::CycleDetected { .. }));
        assert_eq!(issues[1], ValidationIssue::NoOutputNode);
        assert_eq!(issues.len(), 2);
    }
}
