//! Integration tests for the collapsed topology view: imported topologies as
//! single nodes, declared ports on the boundary, and implicit edges for
//! connections that reach into an imported topology.

use fpp_analysis::{Analysis, add_state_enums, check_semantics};
use fpp_core::SourceFile;
use fpp_diagram::ir::{Diagram, DiagramKind, Direction, Edge, TopologyView};

/// Parse `src` and run full semantic analysis, invoking `f` with the resulting
/// [`Analysis`] while the compiler context is installed.
fn with_analysis<R>(src: &str, f: impl FnOnce(&Analysis) -> R) -> R {
    let mut sink = Vec::new();
    let mut ctx = fpp_core::CompilerContext::new(fpp_errors::WriteEmitter::new(&mut sink));
    fpp_core::run(&mut ctx, || {
        let source = SourceFile::new("test.fpp", src.to_string());
        let mut ast = fpp_parser::parse(source, |p| p.trans_unit(), None);
        add_state_enums(&mut ast);
        let mut a = Analysis::new();
        let _ = check_semantics(&mut a, vec![&ast]);
        f(&a)
    })
}

/// A deployment `Top` importing a subtopology `Sub.Subtopology`. `Top` reaches
/// the subtopology both through its declared ports (`dataIn`, `dataOut`) and by
/// naming its instances directly (`Sub.inner`).
const MODEL: &str = r#"
port P

passive component Src {
    output port out: P
    output port out2: P
}

active component Sink {
    async input port in: P
    sync input port in2: P
    output port back: P
}

module Sub {
    instance src: Src base id 0x100
    instance inner: Sink base id 0x200 \
        queue size 10

    topology Subtopology {
        instance src
        instance inner

        port dataIn = inner.in
        port dataOut = src.out2
        port spare = inner.in2

        connections Internal {
            src.out -> inner.in2
        }
    }
}

instance outer: Src base id 0x300
instance hub: Sink base id 0x400 \
    queue size 10

topology Top {
    instance outer
    instance hub
    instance Sub.Subtopology

    connections C {
        outer.out -> Sub.Subtopology.dataIn
        Sub.Subtopology.dataOut -> hub.in
        outer.out2 -> Sub.inner.in2
        Sub.inner.back -> hub.in2
    }
}
"#;

fn node_ids(d: &Diagram) -> Vec<&str> {
    let mut ids: Vec<&str> = d.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort();
    ids
}

fn edge<'a>(d: &'a Diagram, from: &str, to: &str) -> &'a Edge {
    d.edges
        .iter()
        .find(|e| e.from_port == from && e.to_port == to)
        .unwrap_or_else(|| panic!("expected an edge {from} -> {to}; have {:?}", d.edges))
}

#[test]
fn flattened_view_is_the_default_and_unchanged() {
    with_analysis(MODEL, |a| {
        let default = fpp_diagram::lower(a, DiagramKind::Topology, "Top").unwrap();
        let flattened =
            fpp_diagram::lower_view(a, DiagramKind::Topology, "Top", TopologyView::Flattened)
                .unwrap();
        assert_eq!(default, flattened);

        assert_eq!(node_ids(&default), ["Sub.inner", "Sub.src", "hub", "outer"]);
        assert!(default.topology_nodes.is_empty());
        assert!(default.boundary_ports.is_empty());
        // Every connection, including the subtopology's own, resolved to
        // component ports.
        assert_eq!(default.edges.len(), 5);
        assert!(
            default
                .edges
                .iter()
                .all(|e| !e.implicit && e.detail.is_empty())
        );
        edge(&default, "outer.out.0", "Sub.inner.in.0");
        edge(&default, "Sub.src.out2.0", "hub.in.0");
        edge(&default, "Sub.src.out.0", "Sub.inner.in2.0");
    });
}

#[test]
fn collapsed_view_draws_imported_topology_as_one_node() {
    with_analysis(MODEL, |a| {
        let d = fpp_diagram::lower_view(a, DiagramKind::Topology, "Top", TopologyView::Collapsed)
            .unwrap();

        assert_eq!(node_ids(&d), ["hub", "outer"]);
        assert_eq!(d.topology_nodes.len(), 1);
        let sub = &d.topology_nodes[0];
        assert_eq!(sub.id, "Sub.Subtopology");
        assert_eq!(sub.name, "Subtopology");
        assert_eq!(sub.qualified_name, "Sub.Subtopology");

        // The collapsed node carries the subtopology's declared ports, with the
        // direction of the port each aliases.
        let ports: Vec<(&str, Direction)> = sub
            .ports
            .iter()
            .map(|p| (p.id.as_str(), p.direction))
            .collect();
        assert_eq!(
            ports,
            [
                ("Sub.Subtopology.dataIn.0", Direction::Input),
                ("Sub.Subtopology.dataOut.0", Direction::Output),
                ("Sub.Subtopology.spare.0", Direction::Input),
            ]
        );

        // `Top` declares no ports of its own.
        assert!(d.boundary_ports.is_empty());
    });
}

#[test]
fn collapsed_view_edges_use_declared_ports_or_cross_the_boundary() {
    with_analysis(MODEL, |a| {
        let d = fpp_diagram::lower_view(a, DiagramKind::Topology, "Top", TopologyView::Collapsed)
            .unwrap();

        // Connections through a declared port land on that rendered port.
        let e = edge(&d, "outer.out.0", "Sub.Subtopology.dataIn.0");
        assert!(!e.implicit);
        assert!(e.detail.is_empty());
        let e = edge(&d, "Sub.Subtopology.dataOut.0", "hub.in.0");
        assert!(!e.implicit);

        // Connections naming an instance inside the subtopology end on the
        // collapsed node itself, not on a rendered port, and say what they
        // stand for.
        let e = edge(&d, "outer.out2.0", "Sub.Subtopology");
        assert!(e.implicit);
        assert_eq!(e.detail, "outer.out2 -> Sub.inner.in2");
        let e = edge(&d, "Sub.Subtopology", "hub.in2.0");
        assert!(e.implicit);
        assert_eq!(e.detail, "Sub.inner.back -> hub.in2");

        // The subtopology's internal connection is hidden inside its node, and
        // no port was invented on the boundary for the reach-in connections.
        assert_eq!(d.edges.len(), 4);
        assert!(
            d.topology_nodes[0]
                .ports
                .iter()
                .all(|p| p.name != "in2" && p.name != "back")
        );
    });
}

#[test]
fn collapsed_view_keeps_declared_ports_when_pruning_unused() {
    with_analysis(MODEL, |a| {
        let mut d =
            fpp_diagram::lower_view(a, DiagramKind::Topology, "Top", TopologyView::Collapsed)
                .unwrap();
        d.prune_unused_ports();
        // `spare` is declared but unconnected in `Top`; it is still the
        // subtopology's interface.
        assert_eq!(d.topology_nodes[0].ports.len(), 3);
        // Component ports are pruned as usual.
        let outer = d.nodes.iter().find(|n| n.id == "outer").unwrap();
        assert_eq!(outer.ports.len(), 2);
        let hub = d.nodes.iter().find(|n| n.id == "hub").unwrap();
        assert_eq!(hub.ports.len(), 2);
    });
}

#[test]
fn collapsed_view_of_subtopology_draws_its_ports_on_the_boundary() {
    with_analysis(MODEL, |a| {
        let d = fpp_diagram::lower_view(
            a,
            DiagramKind::Topology,
            "Sub.Subtopology",
            TopologyView::Collapsed,
        )
        .unwrap();

        assert_eq!(node_ids(&d), ["Sub.inner", "Sub.src"]);
        assert!(d.topology_nodes.is_empty());

        let boundary: Vec<(&str, Direction)> = d
            .boundary_ports
            .iter()
            .map(|p| (p.id.as_str(), p.direction))
            .collect();
        assert_eq!(
            boundary,
            [
                ("Sub.Subtopology.dataIn.0", Direction::Input),
                ("Sub.Subtopology.dataOut.0", Direction::Output),
                ("Sub.Subtopology.spare.0", Direction::Input),
            ]
        );

        // Each boundary port is joined to the component port it aliases, in
        // the direction data flows.
        let e = edge(&d, "Sub.Subtopology.dataIn.0", "Sub.inner.in.0");
        assert_eq!(e.id, "boundary.dataIn.0");
        assert!(!e.implicit);
        edge(&d, "Sub.src.out2.0", "Sub.Subtopology.dataOut.0");
        edge(&d, "Sub.Subtopology.spare.0", "Sub.inner.in2.0");
        edge(&d, "Sub.src.out.0", "Sub.inner.in2.0");
        assert_eq!(d.edges.len(), 4);
    });
}

#[test]
fn collapsed_connection_group_keeps_participating_topology_nodes() {
    with_analysis(MODEL, |a| {
        let d = fpp_diagram::lower_view(
            a,
            DiagramKind::ConnectionGroup,
            "Top.C",
            TopologyView::Collapsed,
        )
        .unwrap();
        assert_eq!(d.name, "Top.C");
        assert_eq!(node_ids(&d), ["hub", "outer"]);
        assert_eq!(d.topology_nodes.len(), 1);
        assert_eq!(d.edges.len(), 4);
        assert!(d.boundary_ports.is_empty());

        let flattened = fpp_diagram::lower(a, DiagramKind::ConnectionGroup, "Top.C").unwrap();
        assert_eq!(
            node_ids(&flattened),
            ["Sub.inner", "Sub.src", "hub", "outer"]
        );
        assert!(flattened.topology_nodes.is_empty());
    });
}

#[test]
fn sprotty_model_carries_collapsed_elements() {
    with_analysis(MODEL, |a| {
        let model = fpp_diagram::lower_to_smodel_view(
            a,
            DiagramKind::Topology,
            "Top",
            false,
            fpp_diagram::TransitionActionMode::Uml,
            TopologyView::Collapsed,
        )
        .unwrap();
        let children = model["children"].as_array().unwrap();

        let topology_nodes: Vec<_> = children
            .iter()
            .filter(|c| c["type"] == "node:topology")
            .collect();
        assert_eq!(topology_nodes.len(), 1);
        assert_eq!(topology_nodes[0]["id"], "Sub.Subtopology");
        let kids = topology_nodes[0]["children"].as_array().unwrap();
        assert_eq!(kids.iter().filter(|k| k["type"] == "port").count(), 3);

        let implicit: Vec<_> = children
            .iter()
            .filter(|c| c["type"] == "edge" && c["implicit"] == true)
            .collect();
        assert_eq!(implicit.len(), 2);
        assert!(implicit.iter().any(|e| {
            e["sourceId"] == "outer.out2.0"
                && e["targetId"] == "Sub.Subtopology"
                && e["detail"] == "outer.out2 -> Sub.inner.in2"
        }));

        // Explicit edges carry no `implicit`/`detail` keys, as before.
        let explicit = children
            .iter()
            .find(|c| c["type"] == "edge" && c["targetId"] == "Sub.Subtopology.dataIn.0")
            .unwrap();
        assert!(explicit.get("implicit").is_none());
        assert!(explicit.get("detail").is_none());

        let sub_model = fpp_diagram::lower_to_smodel_view(
            a,
            DiagramKind::Topology,
            "Sub.Subtopology",
            false,
            fpp_diagram::TransitionActionMode::Uml,
            TopologyView::Collapsed,
        )
        .unwrap();
        let boundary: Vec<_> = sub_model["children"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "node:boundary")
            .collect();
        assert_eq!(boundary.len(), 3);
        let out = boundary
            .iter()
            .find(|b| b["id"] == "Sub.Subtopology.dataOut.0")
            .unwrap();
        assert_eq!(out["isOutput"], true);
        assert_eq!(out["kind"], "output");
    });
}

#[test]
fn sprotty_flattened_model_is_unchanged_by_the_view_parameter() {
    with_analysis(MODEL, |a| {
        let legacy = fpp_diagram::lower_to_smodel(
            a,
            DiagramKind::Topology,
            "Top",
            true,
            fpp_diagram::TransitionActionMode::Uml,
        )
        .unwrap();
        let flattened = fpp_diagram::lower_to_smodel_view(
            a,
            DiagramKind::Topology,
            "Top",
            true,
            fpp_diagram::TransitionActionMode::Uml,
            TopologyView::Flattened,
        )
        .unwrap();
        assert_eq!(legacy, flattened);
        let children = legacy["children"].as_array().unwrap();
        assert!(children.iter().all(|c| c["type"] != "node:topology"));
        assert!(children.iter().all(|c| c.get("implicit").is_none()));
    });
}

#[test]
fn cli_view_switch_selects_the_collapsed_view() {
    let dir = std::env::temp_dir().join(format!("fpp-diagram-collapsed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("model.fpp");
    std::fs::write(&file, MODEL).unwrap();

    let run = |extra: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_fpp-diagram"))
            .args(["--kind", "topology", "--name", "Top", "--format", "sprotty"])
            .args(extra)
            .arg(&file)
            .output()
            .expect("fpp-diagram runs");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).expect("sprotty JSON")
    };

    let flattened = run(&[]);
    let collapsed = run(&["--view", "collapsed"]);
    std::fs::remove_dir_all(&dir).unwrap();

    let types = |m: &serde_json::Value| -> Vec<String> {
        m["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["type"].as_str().unwrap().to_string())
            .collect()
    };
    assert!(!types(&flattened).iter().any(|t| t == "node:topology"));
    assert_eq!(
        types(&collapsed)
            .iter()
            .filter(|t| *t == "node:topology")
            .count(),
        1
    );
    assert_eq!(
        types(&collapsed)
            .iter()
            .filter(|t| *t == "node:component")
            .count(),
        2
    );
}
