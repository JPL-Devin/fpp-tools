//! Lowering from [`fpp_analysis::Analysis`] into the framework-agnostic diagram
//! [`crate::ir`].
//!
//! This is the single analysis-consuming step. It reads the already-resolved
//! semantic model (components, instances, resolved connections with port
//! numbering) and produces an [`ir::Diagram`]. It performs no layout and knows
//! nothing about any rendering framework.

use crate::ir::{self, Diagram, DiagramKind, Edge, Node, Port, TopologyNode, TopologyView};
use fpp_analysis::Analysis;
use fpp_analysis::semantics::{
    Component, ComponentInstance, Connection, Direction as SemDirection, Endpoint, GeneralKind,
    InterfaceInstance, PortInstance, PortInstanceIdentifier, PortInstanceType, Symbol,
    SymbolInterface, Topology,
};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;

/// Errors that can occur while lowering an element to a diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    /// No element with the requested name and kind was found in the analysis.
    NotFound { kind: DiagramKind, name: String },
    /// The requested connection group does not exist in the topology.
    UnknownConnectionGroup { topology: String, group: String },
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::NotFound { kind, name } => {
                write!(f, "no {kind:?} named `{name}` found in analysis")
            }
            LowerError::UnknownConnectionGroup { topology, group } => {
                write!(f, "topology `{topology}` has no connection group `{group}`")
            }
        }
    }
}

impl std::error::Error for LowerError {}

/// Lower a component definition (by fully qualified name) into a diagram.
pub fn lower_component(a: &Analysis, name: &str) -> Result<Diagram, LowerError> {
    let component = find_component(a, name).ok_or_else(|| LowerError::NotFound {
        kind: DiagramKind::Component,
        name: name.to_string(),
    })?;

    let qualified_name = a.get_qualified_name(&component.symbol);
    let node = component_def_node(a, component, &qualified_name);

    Ok(Diagram {
        kind: DiagramKind::Component,
        name: qualified_name,
        nodes: vec![node],
        edges: vec![],
        topology_nodes: vec![],
        boundary_ports: vec![],
    })
}

/// Lower a topology (by fully qualified name) into a diagram of its component
/// instances and all their connections ([`TopologyView::Flattened`]).
pub fn lower_topology(a: &Analysis, name: &str) -> Result<Diagram, LowerError> {
    lower_topology_view(a, name, TopologyView::Flattened)
}

/// Lower a topology (by fully qualified name) into a diagram, with `view`
/// selecting how imported topologies are drawn.
pub fn lower_topology_view(
    a: &Analysis,
    name: &str,
    view: TopologyView,
) -> Result<Diagram, LowerError> {
    let topology = find_topology(a, name).ok_or_else(|| LowerError::NotFound {
        kind: DiagramKind::Topology,
        name: name.to_string(),
    })?;

    let mut diagram = Diagram {
        kind: DiagramKind::Topology,
        name: topology.qualified_name.clone(),
        nodes: vec![],
        edges: vec![],
        topology_nodes: vec![],
        boundary_ports: vec![],
    };
    match view {
        TopologyView::Flattened => {
            diagram.nodes = instance_nodes(a, topology);
            diagram.edges = topology_edges(a, topology, None);
        }
        TopologyView::Collapsed => {
            let scope = CollapsedScope::new(a, topology);
            diagram.nodes = scope.component_nodes();
            diagram.topology_nodes = scope.topology_nodes();
            diagram.edges = scope.edges(None);
            diagram
                .edges
                .extend(scope.import_alias_edges().into_iter().map(|(_, e)| e));
            let (ports, edges) = scope.boundary();
            diagram.boundary_ports = ports;
            diagram.edges.extend(edges);
        }
    }
    Ok(diagram)
}

/// Lower a single named connection group within a topology into a diagram
/// ([`TopologyView::Flattened`]).
///
/// Only the instances participating in the group's connections are included.
pub fn lower_connection_group(
    a: &Analysis,
    topology_name: &str,
    group: &str,
) -> Result<Diagram, LowerError> {
    lower_connection_group_view(a, topology_name, group, TopologyView::Flattened)
}

/// Lower a single named connection group within a topology into a diagram,
/// with `view` selecting how imported topologies are drawn.
///
/// Only the nodes participating in the group's connections are included. In the
/// collapsed view the topology's own boundary ports are omitted, since they
/// belong to no connection group.
pub fn lower_connection_group_view(
    a: &Analysis,
    topology_name: &str,
    group: &str,
    view: TopologyView,
) -> Result<Diagram, LowerError> {
    let topology = find_topology(a, topology_name).ok_or_else(|| LowerError::NotFound {
        kind: DiagramKind::ConnectionGroup,
        name: topology_name.to_string(),
    })?;

    if !topology.connection_map.contains_key(group) {
        return Err(LowerError::UnknownConnectionGroup {
            topology: topology.qualified_name.clone(),
            group: group.to_string(),
        });
    }

    // Keep only the nodes that participate in this group; a collapsed node
    // participates even when the group's wiring lies wholly inside it.
    let (edges, nodes, topology_nodes) = match view {
        TopologyView::Flattened => {
            let edges = topology_edges(a, topology, Some(group));
            let used_nodes: FxHashSet<&str> = edges
                .iter()
                .flat_map(|e| [node_id_of_port(&e.from_port), node_id_of_port(&e.to_port)])
                .collect();
            let nodes = instance_nodes(a, topology)
                .into_iter()
                .filter(|n| used_nodes.contains(n.id.as_str()))
                .collect();
            (edges, nodes, vec![])
        }
        TopologyView::Collapsed => {
            let scope = CollapsedScope::new(a, topology);
            let mut touched = scope.touched(Some(group));
            // A declared port of a drawn collapsed node brings in what it aliases.
            let alias_edges: Vec<Edge> = scope
                .import_alias_edges()
                .into_iter()
                .filter(|(import, _)| touched.contains(import))
                .map(|(_, e)| e)
                .collect();
            for e in &alias_edges {
                touched.insert(scope.node_of(&e.from_port).to_string());
                touched.insert(scope.node_of(&e.to_port).to_string());
            }
            let nodes = scope
                .component_nodes()
                .into_iter()
                .filter(|n| touched.contains(&n.id))
                .collect();
            let topology_nodes = scope
                .topology_nodes()
                .into_iter()
                .filter(|n| touched.contains(&n.id))
                .collect();
            let mut edges = scope.edges(Some(group));
            edges.extend(alias_edges);
            (edges, nodes, topology_nodes)
        }
    };

    Ok(Diagram {
        kind: DiagramKind::ConnectionGroup,
        name: format!("{}.{}", topology.qualified_name, group),
        nodes,
        edges,
        topology_nodes,
        boundary_ports: vec![],
    })
}

/// Recover the node id from a port id (`<node id>.<port>.<index>`), i.e. strip
/// the final two dot-separated segments.
fn node_id_of_port(port_id: &str) -> &str {
    match (
        port_id.rfind('.'),
        port_id.get(..port_id.rfind('.').unwrap_or(0)),
    ) {
        (Some(_), Some(head)) => match head.rfind('.') {
            Some(cut) => &head[..cut],
            None => head,
        },
        _ => port_id,
    }
}

// --- node construction ------------------------------------------------------

/// Build the single node for a component-definition diagram.
fn component_def_node(a: &Analysis, component: &Component, qualified_name: &str) -> Node {
    Node {
        id: qualified_name.to_string(),
        name: component.symbol.name().data.clone(),
        qualified_name: qualified_name.to_string(),
        class_name: None,
        kind: (&component.node.kind).into(),
        ports: component_ports(a, component, qualified_name),
    }
}

/// Build nodes for every component instance in a topology.
fn instance_nodes(a: &Analysis, topology: &Topology) -> Vec<Node> {
    topology
        .component_instance_map()
        .into_keys()
        .filter_map(|ci| instance_node(a, &ci))
        .collect()
}

/// Build a node for a single component instance, or `None` if its component is
/// unresolved.
fn instance_node(a: &Analysis, ci: &ComponentInstance) -> Option<Node> {
    let component = a.component_map.get(&ci.component_symbol)?;
    Some(Node {
        id: ci.qualified_name.clone(),
        name: ci.get_unqualified_name().to_string(),
        qualified_name: ci.qualified_name.clone(),
        class_name: Some(a.get_qualified_name(&component.symbol)),
        kind: (&component.node.kind).into(),
        ports: component_ports(a, component, &ci.qualified_name),
    })
}

/// Expand all of a component's port instances into per-index [`Port`]s, owned by
/// the node identified by `node_id`.
fn component_ports(a: &Analysis, component: &Component, node_id: &str) -> Vec<Port> {
    let mut ports: Vec<Port> = component
        .port_map()
        .values()
        .flat_map(|pi| expand_port(a, node_id, pi))
        .collect();
    // Deterministic order: by port name, then array index.
    ports.sort_by(|x, y| x.name.cmp(&y.name).then(x.index.cmp(&y.index)));
    ports
}

/// Expand one port instance into one [`Port`] per array index.
fn expand_port(a: &Analysis, node_id: &str, pi: &PortInstance) -> Vec<Port> {
    let name = pi.get_unqualified_name().to_string();
    let direction = match pi.get_direction() {
        Some(SemDirection::Output) => ir::Direction::Output,
        // Internal ports have no direction; treat as input for placement.
        Some(SemDirection::Input) | None => ir::Direction::Input,
    };
    let kind = port_kind(pi);
    let type_name = match pi.get_type() {
        Some(PortInstanceType::DefPort(def)) => {
            Some(a.get_qualified_name(&Symbol::Port(def.clone())))
        }
        Some(PortInstanceType::Serial) | None => None,
    };
    let array_size = pi.get_array_size().max(1);

    (0..array_size)
        .map(|index| {
            let label = if array_size > 1 {
                format!("{name}[{index}]")
            } else {
                name.clone()
            };
            Port {
                id: Port::make_id(node_id, &name, index),
                name: name.clone(),
                label,
                direction,
                kind: kind.clone(),
                index,
                array_size,
                type_name: type_name.clone(),
            }
        })
        .collect()
}

/// Classify a port instance into a rendering-relevant [`ir::PortKind`].
fn port_kind(pi: &PortInstance) -> ir::PortKind {
    match pi {
        PortInstance::General(pi) => match pi.kind {
            GeneralKind::AsyncInput { .. } => ir::PortKind::Async,
            GeneralKind::GuardedInput => ir::PortKind::Guarded,
            GeneralKind::SyncInput => ir::PortKind::Sync,
            GeneralKind::Output => ir::PortKind::Output,
        },
        PortInstance::Special(pi) => ir::PortKind::Special(pi.node.kind.to_string()),
        PortInstance::Internal(_) => ir::PortKind::Internal,
        PortInstance::Topology(pi) => port_kind(&pi.underlying_port),
    }
}

// --- edge construction ------------------------------------------------------

/// Build edges for a topology. When `only_group` is `Some`, only connections in
/// that named graph are included; otherwise all connections are included.
fn topology_edges(a: &Analysis, topology: &Topology, only_group: Option<&str>) -> Vec<Edge> {
    let mut edges = Vec::new();
    for (graph_name, connections) in &topology.connection_map {
        if let Some(group) = only_group
            && group != graph_name
        {
            continue;
        }
        for (i, connection) in connections.iter().enumerate() {
            if let Some(edge) = connection_edge(a, topology, graph_name, connection, i) {
                edges.push(edge);
            }
        }
    }
    edges
}

/// Build a single edge from a resolved connection, resolving both endpoints
/// through any topology-port aliases down to the underlying component ports.
fn connection_edge(
    a: &Analysis,
    topology: &Topology,
    graph_name: &str,
    connection: &Connection,
    seq: usize,
) -> Option<Edge> {
    let from_ep = connection.from.get_underlying_endpoint(a);
    let to_ep = connection.to.get_underlying_endpoint(a);

    // Only connections between component instances render as port-to-port edges.
    if !matches!(
        from_ep.port.interface_instance,
        InterfaceInstance::Component(_)
    ) || !matches!(
        to_ep.port.interface_instance,
        InterfaceInstance::Component(_)
    ) {
        return None;
    }

    let (from_index, to_index) = connection_indices(topology, connection);

    let from_port = Port::make_id(
        &from_ep.port.interface_instance.qualified_name(),
        from_ep.port.port_instance.get_unqualified_name(),
        from_index,
    );
    let to_port = Port::make_id(
        &to_ep.port.interface_instance.qualified_name(),
        to_ep.port.port_instance.get_unqualified_name(),
        to_index,
    );

    Some(Edge {
        id: format!("{graph_name}.connection.{seq}"),
        from_port,
        to_port,
        graph_name: graph_name.to_string(),
        unmatched: connection.is_unmatched,
        implicit: false,
        detail: String::new(),
    })
}

/// The port indices of a connection's endpoints. The resolved (auto-assigned)
/// port numbers are keyed by the original connection; fall back to any explicit
/// number, then to index 0.
fn connection_indices(topology: &Topology, connection: &Connection) -> (i128, i128) {
    let from_index = topology
        .from_port_number_map
        .get(connection)
        .copied()
        .or(connection.from.port_number)
        .unwrap_or(0);
    let to_index = topology
        .to_port_number_map
        .get(connection)
        .copied()
        .or(connection.to.port_number)
        .unwrap_or(0);
    (from_index, to_index)
}

// --- collapsed view ---------------------------------------------------------

/// Where a connection endpoint lands in the collapsed view.
#[derive(Debug, Clone, PartialEq, Eq)]
enum End {
    /// A rendered port: on a direct component instance, or a declared port of a
    /// collapsed topology node.
    Port(String),
    /// The boundary of a collapsed topology node, reached without going through
    /// one of its declared ports.
    Node(String),
}

impl End {
    fn id(&self) -> &str {
        match self {
            End::Port(id) | End::Node(id) => id,
        }
    }
}

/// What the collapsed view of a topology draws: each directly imported topology
/// as one collapsed node standing in for the component instances it owns, and
/// every other component instance as a node of its own.
///
/// An instance is owned by the innermost directly imported topology containing
/// it: of the imports that contain it, those importing another such import are
/// ruled out. An instance the diagrammed topology declares itself, or that two
/// unrelated imports both contain, has no single owner and stays visible as a
/// node of its own, so nothing is hidden inside a box it does not belong to.
struct CollapsedScope<'a> {
    a: &'a Analysis,
    topology: &'a Topology,
    /// Directly imported topologies by qualified name (the collapsed node id).
    imported: BTreeMap<String, &'a Topology>,
    /// Component instance qualified name → collapsed node id that stands for it.
    owner: FxHashMap<String, String>,
}

impl<'a> CollapsedScope<'a> {
    fn new(a: &'a Analysis, topology: &'a Topology) -> Self {
        let imported: BTreeMap<String, &Topology> = topology
            .direct_topologies
            .keys()
            .filter_map(|sym| a.topology_map.get(sym))
            .map(|t| (t.qualified_name.clone(), t))
            .collect();

        let direct: FxHashSet<&str> = topology
            .direct_component_instances
            .keys()
            .filter_map(|sym| a.component_instance_map.get(sym))
            .map(|ci| ci.qualified_name.as_str())
            .collect();

        // Imports containing each instance, innermost first.
        let mut containing: FxHashMap<String, Vec<&Topology>> = FxHashMap::default();
        for imported_topology in imported.values() {
            for ci in imported_topology.component_instance_map().into_keys() {
                if !direct.contains(ci.qualified_name.as_str()) {
                    containing
                        .entry(ci.qualified_name)
                        .or_default()
                        .push(imported_topology);
                }
            }
        }
        let imports = |t: &Topology, other: &Topology| -> bool {
            t.transitive_import_set.iter().any(|sym| {
                a.topology_map.get(sym).map(|d| d.qualified_name.as_str())
                    == Some(&other.qualified_name)
            })
        };
        let owner = containing
            .into_iter()
            .filter_map(|(instance, candidates)| {
                let innermost: Vec<&Topology> = candidates
                    .iter()
                    .copied()
                    .filter(|c| {
                        !candidates
                            .iter()
                            .any(|o| !std::ptr::eq(*c, *o) && imports(c, o))
                    })
                    .collect();
                match innermost.as_slice() {
                    [only] => Some((instance, only.qualified_name.clone())),
                    _ => None,
                }
            })
            .collect();

        CollapsedScope {
            a,
            topology,
            imported,
            owner,
        }
    }

    /// Nodes for the component instances not stood in for by a collapsed node.
    fn component_nodes(&self) -> Vec<Node> {
        self.topology
            .component_instance_map()
            .into_keys()
            .filter(|ci| !self.owner.contains_key(&ci.qualified_name))
            .filter_map(|ci| instance_node(self.a, &ci))
            .collect()
    }

    /// One collapsed node per directly imported topology, carrying its declared
    /// ports.
    fn topology_nodes(&self) -> Vec<TopologyNode> {
        self.imported
            .iter()
            .map(|(id, t)| TopologyNode {
                id: id.clone(),
                name: t.get_name().to_string(),
                qualified_name: id.clone(),
                ports: declared_ports(self.a, t, id),
            })
            .collect()
    }

    /// Where a port instance identifier (as written in a connection or topology
    /// port) lands: a rendered port, or the boundary of a collapsed node.
    fn resolve_end(&self, pii: &PortInstanceIdentifier, index: i128) -> Option<End> {
        let port_name = pii.port_instance.get_unqualified_name();
        match &pii.interface_instance {
            InterfaceInstance::Topology(t) => {
                if self.imported.contains_key(&t.qualified_name) {
                    return Some(End::Port(Port::make_id(
                        &t.qualified_name,
                        port_name,
                        index,
                    )));
                }
                // Not a collapsed node here: follow the alias to what it names.
                let aliased = pii.interface_instance.as_topology(self.a)?;
                let tp = aliased.port_map.get(port_name)?;
                self.resolve_end(&tp.pii, index)
            }
            InterfaceInstance::Component(ci) => match self.owner.get(&ci.qualified_name) {
                Some(node) => Some(End::Node(node.clone())),
                None => Some(End::Port(Port::make_id(
                    &ci.qualified_name,
                    port_name,
                    index,
                ))),
            },
        }
    }

    /// The node an edge end belongs to: the end is either a collapsed node id
    /// or a port id.
    fn node_of<'b>(&self, end: &'b str) -> &'b str {
        if self.imported.contains_key(end) {
            end
        } else {
            node_id_of_port(end)
        }
    }

    /// The connection graphs to draw: all of them, or just `only_group`.
    fn graphs(
        &self,
        only_group: Option<&str>,
    ) -> impl Iterator<Item = (&String, &Vec<Connection>)> {
        self.topology
            .connection_map
            .iter()
            .filter(move |(graph_name, _)| only_group.is_none_or(|g| g == graph_name.as_str()))
    }

    /// Ids of the nodes the connections of the given graphs touch, including
    /// collapsed nodes whose interior wiring they are.
    fn touched(&self, only_group: Option<&str>) -> FxHashSet<String> {
        self.graphs(only_group)
            .flat_map(|(_, connections)| connections)
            .flat_map(|c| {
                let (from_index, to_index) = connection_indices(self.topology, c);
                [
                    self.resolve_end(&as_written(&c.from).port, from_index),
                    self.resolve_end(&as_written(&c.to).port, to_index),
                ]
            })
            .flatten()
            .map(|end| match end {
                End::Port(id) => node_id_of_port(&id).to_string(),
                End::Node(id) => id,
            })
            .collect()
    }

    /// Edges for every connection of the topology, imported ones included,
    /// except those lying wholly inside one collapsed node. When `only_group` is
    /// `Some`, only that named graph is included.
    fn edges(&self, only_group: Option<&str>) -> Vec<Edge> {
        let mut edges: Vec<Edge> = Vec::new();
        // Implicit edges drawn between the same two elements collapse into one.
        let mut by_ends: FxHashMap<(String, String), usize> = FxHashMap::default();
        for (graph_name, connections) in self.graphs(only_group) {
            for (seq, connection) in connections.iter().enumerate() {
                let Some(edge) = self.connection_edge(graph_name, connection, seq) else {
                    continue;
                };
                if !edge.implicit {
                    edges.push(edge);
                    continue;
                }
                let key = (edge.from_port.clone(), edge.to_port.clone());
                match by_ends.get(&key) {
                    Some(&i) => {
                        let detail = &mut edges[i].detail;
                        if !detail.contains(&edge.detail) {
                            detail.push('\n');
                            detail.push_str(&edge.detail);
                        }
                        edges[i].unmatched &= edge.unmatched;
                    }
                    None => {
                        by_ends.insert(key, edges.len());
                        edges.push(edge);
                    }
                }
            }
        }
        edges
    }

    fn connection_edge(
        &self,
        graph_name: &str,
        connection: &Connection,
        seq: usize,
    ) -> Option<Edge> {
        let (from_index, to_index) = connection_indices(self.topology, connection);
        let from = self.resolve_end(&as_written(&connection.from).port, from_index)?;
        let to = self.resolve_end(&as_written(&connection.to).port, to_index)?;

        // A connection between two instances of the same collapsed node lies
        // entirely inside it.
        if let (End::Node(f), End::Node(t)) = (&from, &to)
            && f == t
        {
            return None;
        }

        let implicit = matches!(from, End::Node(_)) || matches!(to, End::Node(_));
        let detail = if implicit {
            format!(
                "{} -> {}",
                underlying_label(
                    self.a,
                    connection.from.get_underlying_endpoint(self.a).port,
                    from_index
                ),
                underlying_label(
                    self.a,
                    connection.to.get_underlying_endpoint(self.a).port,
                    to_index
                ),
            )
        } else {
            String::new()
        };

        Some(Edge {
            id: format!("{graph_name}.connection.{seq}"),
            from_port: from.id().to_string(),
            to_port: to.id().to_string(),
            graph_name: graph_name.to_string(),
            unmatched: connection.is_unmatched,
            implicit,
            detail,
        })
    }

    /// The topology's own declared ports, drawn on the diagram boundary, and
    /// the edges joining each to the port it aliases.
    fn boundary(&self) -> (Vec<Port>, Vec<Edge>) {
        self.alias_edges(self.topology, &self.topology.qualified_name, "boundary")
    }

    /// Edges joining a declared port of a collapsed node to what it aliases
    /// when that lies outside the node: an instance drawn on its own, or one
    /// owned by another collapsed node. Each is paired with the collapsed
    /// node's id.
    fn import_alias_edges(&self) -> Vec<(String, Edge)> {
        self.imported
            .iter()
            .flat_map(|(id, t)| {
                self.alias_edges(t, id, &format!("alias.{id}"))
                    .1
                    .into_iter()
                    .map(move |e| (id.clone(), e))
            })
            .collect()
    }

    /// `topology`'s declared ports as ports of the node `node_id`, and an edge
    /// from each to the port it aliases unless that lies inside the same node.
    fn alias_edges(
        &self,
        topology: &Topology,
        node_id: &str,
        id_prefix: &str,
    ) -> (Vec<Port>, Vec<Edge>) {
        let mut ports = Vec::new();
        let mut edges = Vec::new();
        for port in declared_ports(self.a, topology, node_id) {
            let Some(tp) = topology.port_map.get(&port.name) else {
                continue;
            };
            let Some(inner) = self.resolve_end(&tp.pii, port.index) else {
                continue;
            };
            if inner.id() == node_id {
                ports.push(port);
                continue;
            }
            let implicit = matches!(inner, End::Node(_));
            let detail = if implicit {
                underlying_label(self.a, tp.pii.clone(), port.index)
            } else {
                String::new()
            };
            let (from_port, to_port) = match port.direction {
                ir::Direction::Input => (port.id.clone(), inner.id().to_string()),
                ir::Direction::Output => (inner.id().to_string(), port.id.clone()),
            };
            edges.push(Edge {
                id: format!("{id_prefix}.{}.{}", port.name, port.index),
                from_port,
                to_port,
                graph_name: String::new(),
                unmatched: false,
                implicit,
                detail,
            });
            ports.push(port);
        }
        (ports, edges)
    }
}

/// The endpoint as written in the connection: a resolved endpoint records the
/// topology-port alias it came through, so walk out to the outermost one.
fn as_written(endpoint: &Endpoint) -> &Endpoint {
    let mut written = endpoint;
    while let Some(alias) = &written.topology_port {
        written = alias;
    }
    written
}

/// A topology's declared ports (`port x = inst.p`), expanded per index and owned
/// by the node identified by `node_id`.
fn declared_ports(a: &Analysis, topology: &Topology, node_id: &str) -> Vec<Port> {
    let mut ports: Vec<Port> = topology
        .port_interface
        .port_map
        .values()
        .flat_map(|pi| expand_port(a, node_id, pi))
        .collect();
    ports.sort_by(|x, y| x.name.cmp(&y.name).then(x.index.cmp(&y.index)));
    ports
}

/// `inst.port` or `inst.port[index]` for the component port a topology-port
/// alias ultimately names.
fn underlying_label(a: &Analysis, pii: PortInstanceIdentifier, index: i128) -> String {
    let mut pii = pii;
    while let Some(top) = pii.interface_instance.as_topology(a) {
        match top.port_map.get(pii.port_instance.get_unqualified_name()) {
            Some(tp) => pii = tp.pii.clone(),
            None => break,
        }
    }
    if pii.port_instance.get_array_size() > 1 {
        format!("{}[{index}]", pii.qualified_name())
    } else {
        pii.qualified_name()
    }
}

// --- lookup helpers ---------------------------------------------------------

/// Find a fully resolved topology by its fully qualified name.
fn find_topology<'a>(a: &'a Analysis, name: &str) -> Option<&'a Topology> {
    a.topology_map.values().find(|t| t.qualified_name == name)
}

/// Find a component by its fully qualified name.
fn find_component<'a>(a: &'a Analysis, name: &str) -> Option<&'a Component> {
    a.component_map
        .values()
        .find(|c| a.get_qualified_name(&c.symbol) == name)
}
