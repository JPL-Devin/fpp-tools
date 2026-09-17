//! End-to-end tests for init-specifier (`phase`) navigation: goto-definition
//! into the generated topology C++, hover, and folding.
#![cfg(test)]

use crate::handlers;
use crate::workspace_tests::{fixture_dir, index_workspace};
use lsp_types::{
    FoldingRangeParams, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverParams, Location, Position, TextDocumentIdentifier, TextDocumentPositionParams, Uri,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;

const TOP_FPP: &str = r#"module Fpp {
  module ToCpp {
    enum Phases : U8 {
      configConstants
      configObjects
      instances
      initComponents
      configComponents
      regCommands
    }
  }
}

module Ref {
  passive component C {
    sync input port p: Fw.Ping
  }

  instance c: C base id 0x100 {
    phase Fpp.ToCpp.Phases.configComponents """
      c.configure(1);
      """
    phase Fpp.ToCpp.Phases.configObjects """
      int x = 1;
      """
  }

  instance unused: C base id 0x200 {
    phase Fpp.ToCpp.Phases.configComponents """
      unused.configure(1);
      """
  }

  instance shared: C base id 0x300 {
    phase Fpp.ToCpp.Phases.configConstants """
      enum { SHARED_LIMIT = 3 };
      """
    phase Fpp.ToCpp.Phases.configComponents """
      shared.configure(SHARED_LIMIT);
      """
  }

  topology Ref {
    instance c
    instance shared
  }

  topology Alt {
    instance shared
  }
}
"#;

const PING_FPP: &str = "module Fw {\n  port Ping\n}\n";

const GENERATED_CPP: &str = r#"// Generated
namespace Ref {

  namespace ConfigObjects {

    namespace Ref_c {
      int x = 1;
    }

  }

  void initComponents(const TopologyState& state) {
    c.init();
  }

  void configComponents(const TopologyState& state) {
    c.configure(1);
    shared.configure(SHARED_LIMIT);
  }

}
"#;

const GENERATED_HPP: &str = r#"// Generated
namespace Ref {

  namespace ConfigConstants {
    namespace Ref_shared {
      enum { SHARED_LIMIT = 3 };
    }
  }

}
"#;

const GENERATED_ALT_CPP: &str = r#"// Generated
namespace Ref {

  void configComponents(const TopologyState& state) {
    shared.configure(SHARED_LIMIT);
  }

}
"#;

struct Fixture {
    dir: PathBuf,
    top_uri: String,
    generated_cpp: PathBuf,
    generated_hpp: PathBuf,
    generated_alt_cpp: PathBuf,
}

/// Lay out a project with a `.fpp-lsp` pointing at a build cache that holds
/// `locs.fpp`, `fprime-locations.fprime-util` and a generated `RefTopologyAc.cpp`.
fn fixture(name: &str) -> Fixture {
    let dir = fixture_dir(name);
    let top = dir.join("Top");
    let build = dir.join("build-fprime-automatic-native");
    std::fs::create_dir_all(&top).unwrap();
    std::fs::create_dir_all(build.join("Top")).unwrap();

    std::fs::write(top.join("Top.fpp"), TOP_FPP).unwrap();
    std::fs::write(top.join("Ping.fpp"), PING_FPP).unwrap();
    std::fs::write(
        build.join("locs.fpp"),
        "locate type Fpp.ToCpp.Phases at \"../Top/Top.fpp\"\n\
         locate component Ref.C at \"../Top/Top.fpp\"\n\
         locate instance Ref.c at \"../Top/Top.fpp\"\n\
         locate instance Ref.unused at \"../Top/Top.fpp\"\n\
         locate instance Ref.shared at \"../Top/Top.fpp\"\n\
         locate topology Ref.Ref at \"../Top/Top.fpp\"\n\
         locate topology Ref.Alt at \"../Top/Top.fpp\"\n\
         locate port Fw.Ping at \"../Top/Ping.fpp\"\n",
    )
    .unwrap();
    std::fs::write(
        build.join("fprime-locations.fprime-util"),
        format!("{}\n{}\n", dir.display(), build.display()),
    )
    .unwrap();
    let generated_cpp = build.join("Top").join("RefTopologyAc.cpp");
    std::fs::write(&generated_cpp, GENERATED_CPP).unwrap();
    let generated_hpp = build.join("Top").join("RefTopologyAc.hpp");
    std::fs::write(&generated_hpp, GENERATED_HPP).unwrap();
    let generated_alt_cpp = build.join("Top").join("AltTopologyAc.cpp");
    std::fs::write(&generated_alt_cpp, GENERATED_ALT_CPP).unwrap();
    std::fs::write(
        dir.join(".fpp-lsp"),
        "buildCache: build-fprime-automatic-native\n",
    )
    .unwrap();

    let top_uri = crate::uri::from_file_path(top.join("Top.fpp")).unwrap();
    Fixture {
        dir,
        top_uri,
        generated_cpp,
        generated_hpp,
        generated_alt_cpp,
    }
}

fn position_of(text: &str, needle: &str, occurrence: usize) -> Position {
    let mut idx = 0;
    let mut from = 0;
    for _ in 0..=occurrence {
        idx = text[from..].find(needle).expect("needle not found") + from;
        from = idx + 1;
    }
    let line = text[..idx].matches('\n').count() as u32;
    let line_start = text[..idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    Position {
        line,
        character: (idx - line_start) as u32,
    }
}

fn line_of(text: &str, needle: &str) -> u32 {
    text.lines().position(|l| l.contains(needle)).unwrap() as u32
}

fn params_at(uri: &str, position: Position) -> TextDocumentPositionParams {
    TextDocumentPositionParams {
        text_document: TextDocumentIdentifier {
            uri: Uri::from_str(uri).unwrap(),
        },
        position,
    }
}

fn goto(state: &crate::global_state::GlobalState, uri: &str, position: Position) -> Vec<Location> {
    let resp = fpp_core::run_ref(&state.context, || {
        handlers::handle_goto_definition(
            state,
            GotoDefinitionParams {
                text_document_position_params: params_at(uri, position),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        )
    })
    .unwrap();
    match resp {
        None => vec![],
        Some(GotoDefinitionResponse::Scalar(l)) => vec![l],
        Some(GotoDefinitionResponse::Array(ls)) => ls,
        Some(GotoDefinitionResponse::Link(_)) => panic!("unexpected LocationLink response"),
    }
}

fn hover(
    state: &crate::global_state::GlobalState,
    uri: &str,
    position: Position,
) -> Option<String> {
    let hover: Option<Hover> = fpp_core::run_ref(&state.context, || {
        handlers::handle_hover(
            state,
            HoverParams {
                text_document_position_params: params_at(uri, position),
                work_done_progress_params: Default::default(),
            },
        )
    })
    .unwrap();
    hover.map(|h| match h.contents {
        HoverContents::Markup(m) => m.value,
        HoverContents::Scalar(lsp_types::MarkedString::String(s)) => s,
        other => panic!("unexpected hover contents {other:?}"),
    })
}

fn path_of(location: &Location) -> PathBuf {
    crate::uri::to_file_path(location.uri.as_str()).unwrap()
}

#[test]
fn goto_on_phase_keyword_jumps_to_generated_snippet() {
    let f = fixture("phases_goto_keyword");
    let state = index_workspace(&f.dir);

    let pos = position_of(TOP_FPP, "phase Fpp.ToCpp.Phases.configComponents", 0);
    let locs = goto(&state, &f.top_uri, pos);
    assert_eq!(locs.len(), 1, "{locs:?}");
    assert_eq!(path_of(&locs[0]), f.generated_cpp);
    assert_eq!(
        locs[0].range.start.line,
        line_of(GENERATED_CPP, "c.configure(1);")
    );
}

#[test]
fn goto_inside_code_string_jumps_to_generated_snippet() {
    let f = fixture("phases_goto_code");
    let state = index_workspace(&f.dir);

    let pos = position_of(TOP_FPP, "c.configure(1);", 0);
    let locs = goto(&state, &f.top_uri, pos);
    assert_eq!(locs.len(), 1, "{locs:?}");
    assert_eq!(path_of(&locs[0]), f.generated_cpp);
    assert_eq!(
        locs[0].range.start.line,
        line_of(GENERATED_CPP, "c.configure(1);")
    );

    // `configObjects` lands in the `ConfigObjects::Ref_c` namespace.
    let pos = position_of(TOP_FPP, "int x = 1;", 0);
    let locs = goto(&state, &f.top_uri, pos);
    assert_eq!(locs.len(), 1, "{locs:?}");
    assert_eq!(
        locs[0].range.start.line,
        line_of(GENERATED_CPP, "int x = 1;")
    );
}

#[test]
fn goto_on_phase_name_still_resolves_enum_constant() {
    let f = fixture("phases_goto_enum");
    let state = index_workspace(&f.dir);

    let mut pos = position_of(TOP_FPP, "Phases.configComponents", 0);
    pos.character += "Phases.".len() as u32 + 2;
    let locs = goto(&state, &f.top_uri, pos);
    assert_eq!(locs.len(), 1, "{locs:?}");
    assert_eq!(
        path_of(&locs[0]),
        Path::new(&f.dir).join("Top").join("Top.fpp")
    );
    assert_eq!(
        locs[0].range.start.line,
        line_of(TOP_FPP, "      configComponents")
    );
}

#[test]
fn goto_lists_every_topology_including_the_instance() {
    let f = fixture("phases_goto_multi");
    let state = index_workspace(&f.dir);

    let pos = position_of(TOP_FPP, "shared.configure(SHARED_LIMIT);", 0);
    let mut locs = goto(&state, &f.top_uri, pos);
    locs.sort_by_key(path_of);
    assert_eq!(locs.len(), 2, "{locs:?}");
    assert_eq!(path_of(&locs[0]), f.generated_alt_cpp);
    assert_eq!(
        locs[0].range.start.line,
        line_of(GENERATED_ALT_CPP, "shared.configure(SHARED_LIMIT);")
    );
    assert_eq!(path_of(&locs[1]), f.generated_cpp);
    assert_eq!(
        locs[1].range.start.line,
        line_of(GENERATED_CPP, "shared.configure(SHARED_LIMIT);")
    );

    let text = hover(&state, &f.top_uri, pos).unwrap();
    assert!(text.contains("`Ref.Alt`"), "{text}");
    assert!(text.contains("`Ref.Ref`"), "{text}");
}

#[test]
fn goto_on_config_constants_targets_the_generated_header() {
    let f = fixture("phases_goto_hpp");
    let state = index_workspace(&f.dir);

    // Only `Ref` has a generated header, so `Alt` contributes no target.
    let pos = position_of(TOP_FPP, "enum { SHARED_LIMIT = 3 };", 0);
    let locs = goto(&state, &f.top_uri, pos);
    assert_eq!(locs.len(), 1, "{locs:?}");
    assert_eq!(path_of(&locs[0]), f.generated_hpp);
    assert_eq!(
        locs[0].range.start.line,
        line_of(GENERATED_HPP, "enum { SHARED_LIMIT = 3 };")
    );
}

#[test]
fn goto_without_topology_or_generated_file_yields_nothing() {
    let f = fixture("phases_goto_missing");
    let state = index_workspace(&f.dir);

    // `unused` is not in any topology.
    let pos = position_of(TOP_FPP, "unused.configure(1);", 0);
    assert!(goto(&state, &f.top_uri, pos).is_empty());
    let text = hover(&state, &f.top_uri, pos).unwrap();
    assert!(text.contains("not part of any topology"), "{text}");

    // Generated file removed: nothing to jump to, hover explains why.
    std::fs::remove_file(&f.generated_cpp).unwrap();
    let pos = position_of(TOP_FPP, "c.configure(1);", 0);
    assert!(goto(&state, &f.top_uri, pos).is_empty());
    let text = hover(&state, &f.top_uri, pos).unwrap();
    assert!(text.contains("no `TopologyAc` file found"), "{text}");
    assert!(text.contains("`Ref.Ref`"), "{text}");
}

#[test]
fn hover_on_phase_describes_phase_and_links_generated_code() {
    let f = fixture("phases_hover");
    let state = index_workspace(&f.dir);

    let pos = position_of(TOP_FPP, "phase Fpp.ToCpp.Phases.configComponents", 0);
    let text = hover(&state, &f.top_uri, pos).unwrap();
    assert!(text.contains("`configComponents`"), "{text}");
    assert!(text.contains("ordinal 4"), "{text}");
    assert!(text.contains("`setup()`"), "{text}");
    assert!(
        text.contains("void configComponents(const TopologyState& state)"),
        "{text}"
    );
    assert!(text.contains("**Instance** `Ref.c` : `Ref.C`"), "{text}");
    let expected_link = format!(
        "RefTopologyAc.cpp:{}",
        line_of(GENERATED_CPP, "c.configure(1);") + 1
    );
    assert!(text.contains(&expected_link), "{text}");
    assert!(!text.contains("stale"), "{text}");
}

#[test]
fn hover_warns_when_generated_file_is_older_than_source() {
    let f = fixture("phases_hover_stale");
    let state = index_workspace(&f.dir);

    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let file = std::fs::File::options()
        .write(true)
        .open(&f.generated_cpp)
        .unwrap();
    file.set_modified(old).unwrap();

    let pos = position_of(TOP_FPP, "c.configure(1);", 0);
    let text = hover(&state, &f.top_uri, pos).unwrap();
    assert!(text.contains("build cache may be stale"), "{text}");
}

#[test]
fn hover_on_phase_name_is_the_enum_hover() {
    let f = fixture("phases_hover_enum");
    let state = index_workspace(&f.dir);

    let mut pos = position_of(TOP_FPP, "Phases.configComponents", 0);
    pos.character += "Phases.".len() as u32 + 2;
    let text = hover(&state, &f.top_uri, pos).unwrap();
    assert!(!text.contains("Init phase"), "{text}");
}

#[test]
fn folding_ranges_cover_phase_blocks_only() {
    let f = fixture("phases_folding");
    let state = index_workspace(&f.dir);

    let ranges = handlers::handle_folding_range(
        &state,
        FoldingRangeParams {
            text_document: TextDocumentIdentifier {
                uri: Uri::from_str(&f.top_uri).unwrap(),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(ranges.len(), 5, "{ranges:?}");
    let first = &ranges[0];
    assert_eq!(
        first.start_line,
        line_of(TOP_FPP, "phase Fpp.ToCpp.Phases.configComponents")
    );
    assert_eq!(first.end_line, first.start_line + 2);
    let second = &ranges[1];
    assert_eq!(
        second.start_line,
        line_of(TOP_FPP, "phase Fpp.ToCpp.Phases.configObjects")
    );
    assert_eq!(second.end_line, second.start_line + 2);
}
