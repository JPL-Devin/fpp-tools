//! Integration tests for bus notation: parallel wires between the same two
//! elements bundled into one counted edge.

use fpp_analysis::{Analysis, add_state_enums, check_semantics};
use fpp_core::SourceFile;
use fpp_diagram::SmodelOptions;
use fpp_diagram::ir::{Diagram, DiagramKind, Edge, TopologyView};

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

/// `hub` fans four wires out to `sink` from two connection groups (a bus), one
/// wire back from `sink` to `hub` (the other direction), one wire to `lone` (no
/// bus), and reaches into the imported `Sub.Subtopology` twice without going
/// through a declared port plus once through its declared port.
const MODEL: &str = r#"
port P

passive component Hub {
    output port out: [4] P
    output port aux: [4] P
    sync input port back: P
}

passive component Sink {
    sync input port in: [3] P
    sync input port extra: P
    output port back: P
}

module Sub {
    instance inner: Sink base id 0x100
    topology Subtopology {
        instance inner
        port declared = inner.extra
    }
}

instance hub: Hub base id 0x200
instance sink: Sink base id 0x300
instance lone: Sink base id 0x400

topology Top {
    instance hub
    instance sink
    instance lone
    instance Sub.Subtopology

    connections Fan {
        hub.out[0] -> sink.in[0]
        hub.out[1] -> sink.in[1]
        hub.out[2] -> sink.in[2]
        sink.back -> hub.back
    }

    connections Other {
        hub.aux[0] -> lone.extra
        hub.aux[1] -> sink.extra
        hub.out[3] -> Sub.inner.in[0]
        hub.aux[2] -> Sub.inner.in[1]
        hub.aux[3] -> Sub.Subtopology.declared
    }
}
"#;

fn edge<'a>(d: &'a Diagram, from: &str, to: &str) -> &'a Edge {
    d.edges
        .iter()
        .find(|e| e.from_port == from && e.to_port == to)
        .unwrap_or_else(|| panic!("expected an edge {from} -> {to}; have {:?}", d.edges))
}

#[test]
fn bundling_is_opt_in_and_every_wire_starts_at_count_one() {
    with_analysis(MODEL, |a| {
        let d = fpp_diagram::lower(a, DiagramKind::Topology, "Top").unwrap();
        assert_eq!(d.edges.len(), 9);
        assert!(d.edges.iter().all(|e| e.count == 1));
        // `count` is omitted from the wire format when it is 1.
        let json = serde_json::to_value(&d).unwrap();
        assert!(json["edges"][0].get("count").is_none());
        let back: Diagram = serde_json::from_value(json).unwrap();
        assert_eq!(back, d);
    });
}

#[test]
fn parallel_wires_bundle_into_one_counted_edge_between_nodes() {
    with_analysis(MODEL, |a| {
        let mut d = fpp_diagram::lower(a, DiagramKind::Topology, "Top").unwrap();
        d.bundle_edges();

        // hub -> sink: four wires from two groups become one bus ending on
        // the nodes.
        let bus = edge(&d, "hub", "sink");
        assert_eq!(bus.count, 4);
        assert_eq!(bus.id, "bus.hub.sink");
        assert_eq!(bus.graph_name, "");
        assert!(!bus.implicit);
        assert_eq!(
            bus.detail,
            "hub.out[0] -> sink.in[0]\nhub.out[1] -> sink.in[1]\nhub.out[2] -> sink.in[2]\nhub.aux[1] -> sink.extra"
        );

        // The other direction is a separate, single wire still on its ports.
        let back = edge(&d, "sink.back.0", "hub.back.0");
        assert_eq!(back.count, 1);
        assert!(back.detail.is_empty());

        // A lone wire keeps its ports.
        edge(&d, "hub.aux.0", "lone.extra.0");

        // hub -> Sub.inner (flattened): three wires, all from one group.
        let bus = edge(&d, "hub", "Sub.inner");
        assert_eq!(bus.count, 3);
        assert_eq!(bus.graph_name, "Other");

        assert_eq!(d.edges.len(), 4);
        assert_eq!(d.edges.iter().map(|e| e.count).sum::<u32>(), 9);
    });
}

#[test]
fn bundling_then_pruning_drops_ports_only_the_bus_used() {
    with_analysis(MODEL, |a| {
        let mut d = fpp_diagram::lower(a, DiagramKind::Topology, "Top").unwrap();
        d.bundle_edges();
        d.prune_unused_ports();
        let ports = |node: &str| -> Vec<String> {
            d.nodes
                .iter()
                .find(|n| n.id == node)
                .unwrap()
                .ports
                .iter()
                .map(|p| p.label.clone())
                .collect()
        };
        assert_eq!(ports("hub"), ["aux[0]", "back"]);
        assert_eq!(ports("sink"), ["back"]);
        assert!(ports("Sub.inner").is_empty());
    });
}

#[test]
fn collapsed_view_counts_merged_implicit_wires_and_bundles_onto_the_node() {
    with_analysis(MODEL, |a| {
        let d = fpp_diagram::lower_view(a, DiagramKind::Topology, "Top", TopologyView::Collapsed)
            .unwrap();
        // Two reach-ins from hub.out[3] and hub.aux are distinct ports, so they
        // are two implicit edges before bundling, each of one wire.
        let e = edge(&d, "hub.out.3", "Sub.Subtopology");
        assert_eq!((e.count, e.implicit), (1, true));
        let e = edge(&d, "hub.aux.2", "Sub.Subtopology");
        assert_eq!((e.count, e.implicit), (1, true));
        let e = edge(&d, "hub.aux.3", "Sub.Subtopology.declared.0");
        assert_eq!((e.count, e.implicit), (1, false));

        let mut d = d;
        d.bundle_edges();
        // The three wires into the collapsed node become one bus; since one of
        // them lands on a declared port the bus is not implicit.
        let bus = edge(&d, "hub", "Sub.Subtopology");
        assert_eq!(bus.count, 3);
        assert!(!bus.implicit);
        assert_eq!(
            bus.detail,
            "hub.out[3] -> Sub.inner.in[0]\nhub.aux[2] -> Sub.inner.in[1]\nhub.aux[3] -> Sub.Subtopology.declared"
        );
        // Its declared port stays on the collapsed node even after pruning.
        d.prune_unused_ports();
        assert_eq!(d.topology_nodes[0].ports.len(), 1);
    });
}

#[test]
fn a_bus_of_only_implicit_wires_stays_implicit() {
    with_analysis(
        r#"
port P
passive component Hub { output port out: [2] P }
passive component Sink { sync input port in: [2] P }
module Sub {
    instance inner: Sink base id 0x100
    topology Subtopology { instance inner }
}
instance hub: Hub base id 0x200
topology Top {
    instance hub
    instance Sub.Subtopology
    connections C {
        hub.out[0] -> Sub.inner.in[0]
        hub.out[1] -> Sub.inner.in[1]
    }
}
"#,
        |a| {
            let mut d =
                fpp_diagram::lower_view(a, DiagramKind::Topology, "Top", TopologyView::Collapsed)
                    .unwrap();
            d.bundle_edges();
            assert_eq!(d.edges.len(), 1);
            let bus = &d.edges[0];
            assert_eq!(
                (bus.from_port.as_str(), bus.to_port.as_str()),
                ("hub", "Sub.Subtopology")
            );
            assert_eq!(bus.count, 2);
            assert!(bus.implicit);
        },
    );
}

#[test]
fn sprotty_bus_edges_carry_count_and_a_slash_label() {
    with_analysis(MODEL, |a| {
        let model = fpp_diagram::lower_to_smodel_with(
            a,
            DiagramKind::Topology,
            "Top",
            SmodelOptions {
                bundle_edges: true,
                ..SmodelOptions::default()
            },
        )
        .unwrap();
        let edges: Vec<&serde_json::Value> = model["children"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "edge")
            .collect();
        assert_eq!(edges.len(), 4);

        let bus = edges
            .iter()
            .find(|e| e["sourceId"] == "hub" && e["targetId"] == "sink")
            .unwrap();
        assert_eq!(bus["count"], 4);
        assert_eq!(bus["detail"].as_str().unwrap().lines().count(), 4);
        let label = &bus["children"][0];
        assert_eq!(label["type"], "label:edge");
        assert_eq!(label["text"], "/4");
        assert_eq!(label["edgePlacement"]["position"], 0.5);

        let single = edges
            .iter()
            .find(|e| e["sourceId"] == "sink.back.0")
            .unwrap();
        assert!(single.get("count").is_none());
        assert!(single.get("children").is_none());

        // Without the option nothing is bundled and the model is the legacy one.
        let plain = fpp_diagram::lower_to_smodel_with(
            a,
            DiagramKind::Topology,
            "Top",
            SmodelOptions::default(),
        )
        .unwrap();
        let legacy = fpp_diagram::lower_to_smodel(
            a,
            DiagramKind::Topology,
            "Top",
            false,
            fpp_diagram::TransitionActionMode::Uml,
        )
        .unwrap();
        assert_eq!(plain, legacy);
        assert_eq!(
            plain["children"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c["type"] == "edge")
                .count(),
            9
        );
    });
}

#[test]
fn sprotty_topology_node_names_the_topology_to_open() {
    with_analysis(MODEL, |a| {
        let model = fpp_diagram::lower_to_smodel_with(
            a,
            DiagramKind::Topology,
            "Top",
            SmodelOptions {
                topology_view: TopologyView::Collapsed,
                ..SmodelOptions::default()
            },
        )
        .unwrap();
        let node = model["children"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["type"] == "node:topology")
            .unwrap();
        assert_eq!(node["qualifiedName"], "Sub.Subtopology");
    });
}

#[test]
fn cli_bundle_switch_bundles_wires() {
    let dir = std::env::temp_dir().join(format!("fpp-diagram-bundle-{}", std::process::id()));
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
    let plain = run(&[]);
    let bundled = run(&["--bundle-edges", "--view", "collapsed"]);
    std::fs::remove_dir_all(&dir).unwrap();

    let edges = |m: &serde_json::Value| -> usize {
        m["children"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "edge")
            .count()
    };
    assert_eq!(edges(&plain), 9);
    // hub->sink bus, sink->hub, hub->lone, hub->Sub.Subtopology bus.
    assert_eq!(edges(&bundled), 4);
    assert!(
        bundled["children"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["type"] == "edge" && c["count"] == 4)
    );
}
