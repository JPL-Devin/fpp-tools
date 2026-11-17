//! Integration tests for compact component diagrams: one drawn port per
//! declared port, labelled with its type and array size.

use fpp_analysis::{Analysis, CheckSemantics};
use fpp_core::SourceFile;
use fpp_diagram::ir::{Diagram, DiagramKind, TopologyView};

fn with_analysis<R>(src: &str, f: impl FnOnce(&Analysis) -> R) -> R {
    let mut sink = Vec::new();
    let mut ctx = fpp_core::CompilerContext::new(fpp_errors::WriteEmitter::new(&mut sink));
    fpp_core::run(&mut ctx, || {
        let source = SourceFile::new("test.fpp", src.to_string());
        let mut ast = fpp_parser::parse(source, |p| p.trans_unit(), None);
        let mut a = Analysis::new();
        let _ =
            CheckSemantics::tu_list(&mut a, fpp_fs::FsReader {}, std::slice::from_mut(&mut ast));
        f(&a)
    })
}

const MODEL: &str = r#"
module Fw {
    port Time
}

module M {
    port P

    active component C {
        async input port run: [3] P
        sync input port one: P
        output port out: [2] P
        output port raw: serial
        internal port tick
        time get port timeCaller
    }

    passive component Clock {
        sync input port timeGetPort: Fw.Time
    }

    instance clock: Clock base id 0x300

    instance c: C base id 0x100 queue size 10
    instance d: C base id 0x200 queue size 10

    topology T {
        instance c
        instance d
        instance clock
        time connections instance clock
        connections W {
            c.out[1] -> d.run[2]
        }
    }
}
"#;

fn labels(d: &Diagram) -> Vec<&str> {
    d.nodes[0].ports.iter().map(|p| p.label.as_str()).collect()
}

#[test]
fn component_ports_are_drawn_once_with_type_and_size() {
    with_analysis(MODEL, |a| {
        let mut d = fpp_diagram::lower(a, DiagramKind::Component, "M.C").unwrap();
        assert_eq!(d.nodes[0].ports.len(), 9, "one element per array index");
        d.compact_ports();
        assert_eq!(
            labels(&d),
            vec![
                "one: M.P",
                "out: [2] M.P",
                "raw: serial",
                "run: [3] M.P",
                "tick",
                "timeCaller: Fw.Time"
            ]
        );
        // The surviving element keeps the id of index 0.
        assert!(d.nodes[0].ports.iter().all(|p| p.id.ends_with(".0")));
    });
}

#[test]
fn topology_diagrams_are_left_alone() {
    with_analysis(MODEL, |a| {
        let mut d = fpp_diagram::lower(a, DiagramKind::Topology, "M.T").unwrap();
        let before = d.clone();
        d.compact_ports();
        assert_eq!(
            d, before,
            "edges name array elements, so none may be dropped"
        );
    });
}

#[test]
fn smodel_option_compacts_component_labels() {
    with_analysis(MODEL, |a| {
        let json = fpp_diagram::lower_to_smodel_with(
            a,
            DiagramKind::Component,
            "M.C",
            fpp_diagram::SmodelOptions {
                compact_ports: true,
                topology_view: TopologyView::Flattened,
                ..Default::default()
            },
        )
        .unwrap()
        .to_string();
        assert!(json.contains("run: [3] M.P"));
        assert!(!json.contains("run[1]"));
    });
}

#[test]
fn pattern_connections_can_be_left_out() {
    with_analysis(MODEL, |a| {
        let mut d = fpp_diagram::lower(a, DiagramKind::Topology, "M.T").unwrap();
        let groups = |d: &Diagram| {
            let mut g: Vec<String> = d.edges.iter().map(|e| e.graph_name.clone()).collect();
            g.sort();
            g
        };
        assert_eq!(groups(&d), vec!["Time", "Time", "W"]);
        d.remove_pattern_connections();
        assert_eq!(groups(&d), vec!["W"]);
    });
}

const COMMAND_MODEL: &str = r#"
module Fw {
    port Cmd
    port CmdReg
    port CmdResponse
}

module M {
    passive component Dispatcher {
        sync input port regIn: Fw.CmdReg
        output port cmdOut: Fw.Cmd
        sync input port responseIn: Fw.CmdResponse
    }

    passive component Worker {
        command recv port cmdIn
        command reg port cmdRegOut
        command resp port cmdResponseOut
        sync command GO
    }

    instance dispatcher: Dispatcher base id 0x100
    instance worker: Worker base id 0x200

    topology T {
        instance dispatcher
        instance worker
        command connections instance dispatcher
    }
}
"#;

#[test]
fn all_three_command_pattern_graphs_are_left_out() {
    with_analysis(COMMAND_MODEL, |a| {
        let mut d = fpp_diagram::lower(a, DiagramKind::Topology, "M.T").unwrap();
        let mut groups: Vec<&str> = d.edges.iter().map(|e| e.graph_name.as_str()).collect();
        groups.sort();
        assert_eq!(
            groups,
            vec!["Command", "CommandRegistration", "CommandResponse"]
        );
        d.remove_pattern_connections();
        assert!(d.edges.is_empty());
    });
}
