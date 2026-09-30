//! FPP diagram lowering.
//!
//! This crate turns the FPP semantic analysis ([`fpp_analysis::Analysis`]) into
//! diagrams, in two stages:
//!
//! 1. **Analysis → IR** ([`lower`]): a framework-agnostic [`ir::Diagram`] made of
//!    component/instance nodes, expanded typed ports, and port-to-port edges.
//! 2. **IR → renderer model** ([`sprotty`]): a sprotty `SModel` JSON tree that a
//!    sprotty host lays out (with ELK) and renders.
//!
//! The IR seam keeps the analysis walk independent of any single rendering
//! framework — a new renderer only needs a new IR consumer, not a new analysis
//! pass.

pub mod ir;
pub mod layout;
pub mod lower;
pub mod lower_sm;
pub mod mermaid;
pub mod sprotty;

pub use ir::{Diagram, DiagramKind, StateMachineDiagram, TopologyView, TransitionActionMode};
pub use layout::SmLayout;
pub use lower::LowerError;

use fpp_analysis::Analysis;

/// Lower a port-graph element (component / topology / connection group) into a
/// diagram IR. For [`DiagramKind::ConnectionGroup`], `name` must be
/// `<topology>.<group>`.
///
/// [`DiagramKind::StateMachine`] is not a port graph and is not handled here;
/// use [`lower_state_machine`] (or [`lower_to_smodel`], which dispatches).
pub fn lower(a: &Analysis, kind: DiagramKind, name: &str) -> Result<Diagram, LowerError> {
    lower_view(a, kind, name, TopologyView::Flattened)
}

/// Like [`lower`], with `view` selecting how topology and connection-group
/// diagrams treat imported topologies (ignored for components).
pub fn lower_view(
    a: &Analysis,
    kind: DiagramKind,
    name: &str,
    view: TopologyView,
) -> Result<Diagram, LowerError> {
    match kind {
        DiagramKind::Component => lower::lower_component(a, name),
        DiagramKind::Topology => lower::lower_topology_view(a, name, view),
        DiagramKind::ConnectionGroup => {
            let (topology, group) = split_group_name(name);
            lower::lower_connection_group_view(a, topology, group, view)
        }
        DiagramKind::StateMachine => Err(LowerError::NotFound {
            kind,
            name: name.to_string(),
        }),
    }
}

/// Lower a state machine (by fully qualified name) into a diagram IR.
///
/// `mode` selects how transition actions are presented on edges; see
/// [`TransitionActionMode`].
pub fn lower_state_machine(
    a: &Analysis,
    name: &str,
    mode: TransitionActionMode,
) -> Result<StateMachineDiagram, LowerError> {
    lower_sm::lower_state_machine(a, name, mode)
}

/// Lower a state machine (by fully qualified name) directly to Mermaid
/// `stateDiagram-v2` source text.
pub fn lower_state_machine_to_mermaid(
    a: &Analysis,
    name: &str,
    mode: TransitionActionMode,
) -> Result<String, LowerError> {
    let diagram = lower_state_machine(a, name, mode)?;
    Ok(mermaid::state_machine_to_mermaid(&diagram, mode))
}

/// Options for lowering an element to a sprotty model; see
/// [`lower_to_smodel_with`]. The default is the plain flattened view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SmodelOptions {
    /// Prune ports not referenced by any connection (a no-op for component and
    /// state machine diagrams). See [`Diagram::prune_unused_ports`].
    pub hide_unused_ports: bool,
    /// How state machine transition actions are presented.
    pub transition_action_mode: TransitionActionMode,
    /// How topology and connection-group diagrams treat imported topologies.
    pub topology_view: TopologyView,
    /// Bundle parallel wires between the same two elements into one bus edge
    /// carrying a wire count. See [`Diagram::bundle_edges`].
    pub bundle_edges: bool,
}

/// Lower an element directly to a sprotty `SModel` JSON value.
///
/// When `hide_unused_ports` is set, ports not referenced by any connection are
/// pruned (a no-op for component and state machine diagrams). See
/// [`Diagram::prune_unused_ports`]. Topologies use the flattened view; see
/// [`lower_to_smodel_with`] for the other options.
pub fn lower_to_smodel(
    a: &Analysis,
    kind: DiagramKind,
    name: &str,
    hide_unused_ports: bool,
    mode: TransitionActionMode,
) -> Result<serde_json::Value, LowerError> {
    lower_to_smodel_with(
        a,
        kind,
        name,
        SmodelOptions {
            hide_unused_ports,
            transition_action_mode: mode,
            ..SmodelOptions::default()
        },
    )
}

/// Like [`lower_to_smodel`], with every option of [`SmodelOptions`].
pub fn lower_to_smodel_with(
    a: &Analysis,
    kind: DiagramKind,
    name: &str,
    options: SmodelOptions,
) -> Result<serde_json::Value, LowerError> {
    if kind == DiagramKind::StateMachine {
        let diagram = lower_state_machine(a, name, options.transition_action_mode)?;
        return Ok(sprotty::state_machine_to_smodel_json(&diagram));
    }
    let mut diagram = lower_view(a, kind, name, options.topology_view)?;
    if options.bundle_edges {
        diagram.bundle_edges();
    }
    if options.hide_unused_ports {
        diagram.prune_unused_ports();
    }
    Ok(sprotty::to_smodel_json(&diagram))
}

/// Split a `<topology>.<group>` connection-group name into its parts. The group
/// is the final dot-separated segment.
fn split_group_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(idx) => (&name[..idx], &name[idx + 1..]),
        None => (name, ""),
    }
}
