//! Navigation support for init specifiers (`phase <Phase> """ code """`).
//!
//! fpp-to-cpp splices the code snippet of every init specifier into the
//! topology's generated `<Top>TopologyAc.{cpp,hpp}` without any marker pointing
//! back at the FPP source. This module bridges the two: it identifies the init
//! specifier under the cursor, finds the topologies that include its instance,
//! locates their generated files in the F´ build cache, and finds the line the
//! snippet was emitted at so the editor can jump straight to it.
//!
//! The phase table mirrors `CppWriter.Phases` in fpp-to-cpp and the "Execution
//! Phases" table of the FPP users guide (Defining Component Instances).

use crate::global_state::{GlobalState, Workspace};
use crate::util::{node_to_location, nodes_at_offset};
use fpp_analysis::semantics::{SymbolInterface, Topology};
use fpp_ast::{AstNode, DefComponentInstance, Node, SpecInit};
use fpp_core::{BytePos, Spanned};
use lsp_types::{Hover, HoverContents, Location, MarkupContent, MarkupKind, Position, Range, Uri};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Which generated file a phase is emitted into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratedFile {
    Hpp,
    Cpp,
}

impl GeneratedFile {
    fn extension(self) -> &'static str {
        match self {
            GeneratedFile::Hpp => "hpp",
            GeneratedFile::Cpp => "cpp",
        }
    }
}

/// Where in the generated file the snippet of a phase lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// `namespace <Ns> { namespace <Instance> { <code> } }`
    Namespace(&'static str),
    /// Anonymous namespace holding the instance definitions.
    Instances,
    /// File-private helper function `void <phase>(<params>)`.
    Function(&'static str),
}

/// When the generated code runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Static, file-scope declarations.
    Static,
    Setup,
    Teardown,
}

#[derive(Debug)]
pub struct PhaseInfo {
    pub name: &'static str,
    pub ordinal: i128,
    pub description: &'static str,
    pub default_code: &'static str,
    pub file: GeneratedFile,
    pub placement: Placement,
    pub stage: Stage,
}

/// The init phases known to fpp-to-cpp, in ordinal order.
pub const PHASES: [PhaseInfo; 13] = [
    PhaseInfo {
        name: "configConstants",
        ordinal: 0,
        description: "C++ constants for use in constructing and initializing the instance.",
        default_code: "None.",
        file: GeneratedFile::Hpp,
        placement: Placement::Namespace("ConfigConstants"),
        stage: Stage::Static,
    },
    PhaseInfo {
        name: "configObjects",
        ordinal: 1,
        description: "Statically declared C++ objects for use in constructing and initializing the instance.",
        default_code: "None.",
        file: GeneratedFile::Cpp,
        placement: Placement::Namespace("ConfigObjects"),
        stage: Stage::Static,
    },
    PhaseInfo {
        name: "instances",
        ordinal: 2,
        description: "A constructor call for an instance that has a non-standard constructor format.",
        default_code: "The standard constructor call for the instance.",
        file: GeneratedFile::Cpp,
        placement: Placement::Instances,
        stage: Stage::Setup,
    },
    PhaseInfo {
        name: "initComponents",
        ordinal: 3,
        description: "Initialization code for an instance that has a non-standard initialization format.",
        default_code: "The standard call to `init` for the instance.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function("const TopologyState& state"),
        stage: Stage::Setup,
    },
    PhaseInfo {
        name: "configComponents",
        ordinal: 4,
        description: "Implementation-specific configuration code for the instance.",
        default_code: "None.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function("const TopologyState& state"),
        stage: Stage::Setup,
    },
    PhaseInfo {
        name: "regCommands",
        ordinal: 5,
        description: "Code for registering the commands of the instance (if any) with the command dispatcher. Required only for a non-standard command registration format.",
        default_code: "The standard call to `regCommands` if the instance has commands; otherwise none.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function(""),
        stage: Stage::Setup,
    },
    PhaseInfo {
        name: "readParameters",
        ordinal: 6,
        description: "Code for reading parameters from a file. Ordinarily used only by the parameter database instance.",
        default_code: "None.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function(""),
        stage: Stage::Setup,
    },
    PhaseInfo {
        name: "loadParameters",
        ordinal: 7,
        description: "Code for loading parameter values from the parameter database. Required only for a non-standard parameter-loading format.",
        default_code: "The standard call to `loadParameters` if the instance has parameters; otherwise none.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function(""),
        stage: Stage::Setup,
    },
    PhaseInfo {
        name: "startTasks",
        ordinal: 8,
        description: "Code for starting the task (if any) of the instance.",
        default_code: "The standard call to `start` if the instance is an active component; otherwise none.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function("const TopologyState& state"),
        stage: Stage::Setup,
    },
    PhaseInfo {
        name: "stopTasks",
        ordinal: 9,
        description: "Code for stopping the task (if any) of the instance.",
        default_code: "The standard call to `exit` if the instance is an active component; otherwise none.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function("const TopologyState& state"),
        stage: Stage::Teardown,
    },
    PhaseInfo {
        name: "freeThreads",
        ordinal: 10,
        description: "Code for freeing the thread associated with the instance.",
        default_code: "The standard call to `join` if the instance is an active component; otherwise none.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function("const TopologyState& state"),
        stage: Stage::Teardown,
    },
    PhaseInfo {
        name: "tearDownComponents",
        ordinal: 11,
        description: "Code for deallocating the allocated memory (if any) associated with the instance.",
        default_code: "None.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function("const TopologyState& state"),
        stage: Stage::Teardown,
    },
    PhaseInfo {
        name: "deinitComponents",
        ordinal: 12,
        description: "Code for performing non-standard deinitialization on the instance, releasing resources acquired during `initComponents`.",
        default_code: "The standard call to `deinit` for the instance.",
        file: GeneratedFile::Cpp,
        placement: Placement::Function("const TopologyState& state"),
        stage: Stage::Teardown,
    },
];

pub fn phase_info(ordinal: i128) -> Option<&'static PhaseInfo> {
    PHASES.iter().find(|p| p.ordinal == ordinal)
}

impl PhaseInfo {
    /// The C++ signature (or namespace path) the snippet is emitted into, for
    /// display purposes.
    pub fn signature(&self, instance_cpp_name: &str) -> String {
        match self.placement {
            Placement::Namespace(ns) => format!("namespace {ns}::{instance_cpp_name}"),
            Placement::Instances => "anonymous namespace (instance definitions)".to_string(),
            Placement::Function(params) => format!("void {}({params})", self.name),
        }
    }
}

/// The init specifier under the cursor, together with the instance defining it.
pub struct InitSpecAt<'a> {
    pub spec: &'a SpecInit,
    pub instance: &'a DefComponentInstance,
}

/// Find the init specifier at `offset`, if the cursor is on the `phase` keyword
/// or inside the code string. The phase expression itself is excluded so that
/// hover/definition on it keeps resolving to the `Fpp.ToCpp.Phases` constant.
pub fn init_spec_at_offset<'a>(
    state: &'a GlobalState,
    document: &Uri,
    offset: BytePos,
) -> Option<InitSpecAt<'a>> {
    let nodes = nodes_at_offset(state, document, offset)?;

    let spec = nodes.iter().find_map(|n| match n {
        Node::SpecInit(spec) => Some(*spec),
        _ => None,
    })?;
    let instance = nodes.iter().find_map(|n| match n {
        Node::DefComponentInstance(def) => Some(*def),
        _ => None,
    })?;

    let phase_span = spec.phase.span();
    let (start, end) = (phase_span.start().pos(), phase_span.end().pos());
    if offset >= start && offset <= end {
        return None;
    }

    Some(InitSpecAt { spec, instance })
}

/// A location in a generated topology file where an init snippet landed.
#[derive(Debug, Clone)]
pub struct GeneratedTarget {
    /// Fully qualified topology name (e.g. `Ref.Ref`).
    pub topology: String,
    pub path: PathBuf,
    /// Zero-based line.
    pub line: u32,
    /// `true` when the snippet text itself was found; `false` when we fell back
    /// to the phase section (function/namespace) or the top of the file.
    pub exact: bool,
    /// The generated file is older than the FPP source containing the snippet.
    pub stale: bool,
}

impl GeneratedTarget {
    pub fn location(&self) -> Option<Location> {
        let uri = crate::uri::from_file_path(&self.path).ok()?;
        let pos = Position {
            line: self.line,
            character: 0,
        };
        Some(Location {
            uri: Uri::from_str(&uri).ok()?,
            range: Range {
                start: pos,
                end: pos,
            },
        })
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// Why no generated target could be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unresolved {
    /// The phase ordinal is not one fpp-to-cpp knows.
    UnknownPhase(i128),
    /// The instance is not part of any resolved topology.
    NoTopology,
    /// No build cache directory is configured (`.fpp-lsp` has no `buildCache`/`locs`).
    NoBuildCache,
    /// Topologies were found but none has a generated file in the build cache.
    NotGenerated(Vec<String>),
}

/// Everything known about an init specifier, resolved from the analysis.
pub struct PhaseContext {
    pub ordinal: i128,
    pub phase: Option<&'static PhaseInfo>,
    pub instance_qualified_name: String,
    pub component_qualified_name: Option<String>,
    /// The snippet with FPP escapes removed, as fpp-to-cpp emits it.
    pub code: String,
    /// The FPP source file holding the init specifier.
    pub source_path: Option<PathBuf>,
}

pub fn phase_context(state: &GlobalState, at: &InitSpecAt<'_>) -> Option<PhaseContext> {
    let analysis = &state.analysis;
    let ordinal = analysis.get_int_value(at.spec.phase.node_id)?;
    let symbol = analysis.symbol_map.get(&at.instance.id())?;
    let instance_qualified_name = analysis.get_qualified_name(symbol);
    let component_qualified_name = analysis
        .component_instance_map
        .get(symbol)
        .map(|ci| analysis.get_qualified_name(&ci.component_symbol));
    let source_path = crate::uri::to_file_path(node_to_location(state, at.spec.id()).uri.as_str());

    Some(PhaseContext {
        ordinal,
        phase: phase_info(ordinal),
        instance_qualified_name,
        component_qualified_name,
        code: unescape(&at.spec.code.data),
        source_path,
    })
}

/// Remove FPP string escapes (`\x` -> `x`), matching the compiler's lexer.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The C++ identifier fpp-to-cpp uses for an instance in `ConfigConstants`/
/// `ConfigObjects` namespaces: the qualified name with `.` replaced by `_`.
pub fn instance_cpp_name(qualified_name: &str) -> String {
    qualified_name.replace('.', "_")
}

/// The topologies whose (transitive) instance set contains the instance.
pub fn topologies_containing<'a>(
    state: &'a GlobalState,
    instance_qualified_name: &str,
) -> Vec<&'a Topology> {
    let mut tops: Vec<&Topology> = state
        .analysis
        .topology_map
        .values()
        .filter(|t| {
            t.component_instance_map()
                .iter()
                .any(|(ci, _)| ci.qualified_name == instance_qualified_name)
        })
        .collect();
    tops.sort_by(|a, b| a.name.cmp(&b.name));
    tops
}

/// Resolve the generated locations of an init snippet across every topology
/// that includes its instance.
pub fn generated_targets(
    state: &GlobalState,
    ctx: &PhaseContext,
) -> Result<Vec<GeneratedTarget>, Unresolved> {
    let phase = ctx.phase.ok_or(Unresolved::UnknownPhase(ctx.ordinal))?;
    let topologies = topologies_containing(state, &ctx.instance_qualified_name);
    if topologies.is_empty() {
        return Err(Unresolved::NoTopology);
    }
    let build_cache = build_cache_dir(state).ok_or(Unresolved::NoBuildCache)?;
    let source_mtime = ctx
        .source_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok());

    let mut targets = vec![];
    for topology in &topologies {
        let top_source = crate::uri::to_file_path(
            node_to_location(state, topology.symbol.name().id())
                .uri
                .as_str(),
        );
        let top_name = topology.symbol.name().data.as_str();
        let Some(path) = generated_file_for_topology(
            &build_cache,
            top_source.as_deref().and_then(Path::parent),
            top_name,
            phase.file,
        ) else {
            continue;
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let stale = match (
            source_mtime,
            std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok()),
        ) {
            (Some(src), Some(generated)) => generated < src,
            _ => false,
        };
        let (line, exact) = locate_snippet(
            &contents,
            phase,
            &instance_cpp_name(&ctx.instance_qualified_name),
            &ctx.code,
        );
        targets.push(GeneratedTarget {
            topology: topology.name.clone(),
            path,
            line,
            exact,
            stale,
        });
    }

    if targets.is_empty() {
        return Err(Unresolved::NotGenerated(
            topologies.iter().map(|t| t.name.clone()).collect(),
        ));
    }
    Ok(targets)
}

/// The build cache directory: the parent of the `locs.fpp` the workspace was
/// indexed from (`.fpp-lsp` `buildCache` resolves to `<buildCache>/locs.fpp`).
pub fn build_cache_dir(state: &GlobalState) -> Option<PathBuf> {
    match &state.workspace {
        Workspace::LocsFile(uri) => {
            let locs = crate::uri::to_file_path(uri.as_str())?;
            locs.parent().map(Path::to_path_buf)
        }
        _ => None,
    }
}

/// Name of the file `fprime-util` writes into a build cache listing
/// `(source dir, build dir)` pairs, one path per line.
const FPRIME_LOCATIONS_FILE: &str = "fprime-locations.fprime-util";

/// Directories skipped when falling back to a scan of the build cache.
const SKIPPED_DIRS: [&str; 3] = ["CMakeFiles", "lib", "bin"];
const MAX_SCAN_DEPTH: usize = 12;

/// Locate `<Top>TopologyAc.<ext>` for a topology defined in `topology_dir`.
///
/// The source directory is mapped into the build cache through the
/// `fprime-locations.fprime-util` pairs; if that fails, the build cache is
/// scanned for a file with the expected name.
pub fn generated_file_for_topology(
    build_cache: &Path,
    topology_dir: Option<&Path>,
    top_name: &str,
    file: GeneratedFile,
) -> Option<PathBuf> {
    let file_name = format!("{top_name}TopologyAc.{}", file.extension());

    if let Some(src_dir) = topology_dir {
        let pairs = std::fs::read_to_string(build_cache.join(FPRIME_LOCATIONS_FILE))
            .map(|s| parse_fprime_locations(&s))
            .unwrap_or_default();
        if let Some(build_dir) = map_source_to_build(&pairs, src_dir) {
            let candidate = build_dir.join(&file_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    find_file_named(build_cache, &file_name, 0)
}

/// Parse `fprime-locations.fprime-util` into `(source root, build root)` pairs.
pub fn parse_fprime_locations(contents: &str) -> Vec<(PathBuf, PathBuf)> {
    let lines: Vec<&str> = contents
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .chunks_exact(2)
        .map(|pair| (PathBuf::from(pair[0]), PathBuf::from(pair[1])))
        .collect()
}

/// Map a source directory into its build directory using the longest matching
/// source root.
pub fn map_source_to_build(pairs: &[(PathBuf, PathBuf)], source_dir: &Path) -> Option<PathBuf> {
    pairs
        .iter()
        .filter_map(|(src, build)| {
            source_dir
                .strip_prefix(src)
                .ok()
                .map(|rel| (src.as_os_str().len(), build.join(rel)))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, path)| path)
}

fn find_file_named(dir: &Path, file_name: &str, depth: usize) -> Option<PathBuf> {
    if depth > MAX_SCAN_DEPTH {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if !SKIPPED_DIRS.iter().any(|s| name == *s) {
                subdirs.push(path);
            }
        } else if entry.file_name() == file_name {
            return Some(path);
        }
    }
    subdirs
        .into_iter()
        .find_map(|d| find_file_named(&d, file_name, depth + 1))
}

/// Find the line the snippet was emitted at in a generated file.
///
/// Returns `(line, exact)`: the zero-based line of the first snippet line when
/// the snippet text is found inside the phase's section, otherwise the first
/// line of the section (`exact == false`), otherwise line 0.
pub fn locate_snippet(
    generated: &str,
    phase: &PhaseInfo,
    instance_cpp_name: &str,
    code: &str,
) -> (u32, bool) {
    let lines: Vec<&str> = generated.lines().collect();
    let section = phase_section(&lines, phase, instance_cpp_name);
    let (start, end) = section.unwrap_or((0, lines.len()));

    if let Some(line) = find_lines(&lines[start..end], code) {
        return ((start + line) as u32, true);
    }
    // The snippet may have been edited since the last build; fall back to the
    // section, then to a whole-file search.
    if let Some((start, _)) = section {
        if let Some(line) = find_lines(&lines, code) {
            return (line as u32, true);
        }
        return (start as u32, false);
    }
    (0, false)
}

/// The `[start, end)` line range of the phase's section in the generated file.
fn phase_section(
    lines: &[&str],
    phase: &PhaseInfo,
    instance_cpp_name: &str,
) -> Option<(usize, usize)> {
    match phase.placement {
        Placement::Function(_) => {
            let header = format!("void {}(", phase.name);
            let start = lines
                .iter()
                .position(|l| l.trim_start().starts_with(&header))?;
            Some((start, block_end(lines, start)))
        }
        Placement::Namespace(ns) => {
            let outer = format!("namespace {ns} {{");
            let outer_start = lines.iter().position(|l| l.trim() == outer)?;
            let outer_end = block_end(lines, outer_start);
            let inner = format!("namespace {instance_cpp_name} {{");
            let inner_start = lines[outer_start..outer_end]
                .iter()
                .position(|l| l.trim() == inner)
                .map(|i| outer_start + i)?;
            Some((inner_start, block_end(lines, inner_start)))
        }
        Placement::Instances => {
            // Instance definitions precede the configuration objects and helper
            // functions; bound the search there when possible.
            let end = lines
                .iter()
                .position(|l| l.trim() == "namespace ConfigObjects {")
                .unwrap_or(lines.len());
            Some((0, end))
        }
    }
}

/// Index one past the line closing the brace block opened on `start`.
fn block_end(lines: &[&str], start: usize) -> usize {
    let mut depth: i32 = 0;
    let mut opened = false;
    for (i, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            match c {
                '{' => {
                    depth += 1;
                    opened = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }
        if opened && depth <= 0 {
            return i + 1;
        }
    }
    lines.len()
}

/// Find `code` (compared line by line, whitespace-trimmed, blank lines
/// ignored) in `lines`; returns the index of the first matching line.
fn find_lines(lines: &[&str], code: &str) -> Option<usize> {
    let needle: Vec<&str> = code
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if needle.is_empty() {
        return None;
    }
    let hay: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| (i, l.trim()))
        .filter(|(_, l)| !l.is_empty())
        .collect();
    hay.windows(needle.len())
        .find(|w| w.iter().zip(&needle).all(|((_, a), b)| a == b))
        .map(|w| w[0].0)
}

/// Hover contents for an init specifier.
pub fn hover_for_init_spec(state: &GlobalState, at: &InitSpecAt<'_>) -> Option<Hover> {
    let ctx = phase_context(state, at)?;
    let mut md: Vec<String> = vec![];

    match ctx.phase {
        Some(phase) => {
            let stage = match phase.stage {
                Stage::Static => "static, file scope".to_string(),
                Stage::Setup => "runs during `setup()`".to_string(),
                Stage::Teardown => "runs during `teardown()`".to_string(),
            };
            md.push(format!(
                "**Init phase** `{}` — `Fpp.ToCpp.Phases` ordinal {}, {stage}",
                phase.name, phase.ordinal
            ));
            md.push(String::new());
            md.push(phase.description.to_string());
            md.push(String::new());
            md.push(format!("Default code: {}", phase.default_code));
            md.push(String::new());
            let file = match phase.file {
                GeneratedFile::Hpp => "TopologyAc.hpp",
                GeneratedFile::Cpp => "TopologyAc.cpp",
            };
            md.push(format!(
                "Generated into `{}` in `<Top>{file}`.",
                phase.signature(&instance_cpp_name(&ctx.instance_qualified_name))
            ));
        }
        None => {
            md.push(format!(
                "**Init phase** ordinal {} — not a phase known to fpp-to-cpp (`Fpp.ToCpp.Phases` defines 0..12).",
                ctx.ordinal
            ));
        }
    }

    md.push(String::new());
    match &ctx.component_qualified_name {
        Some(component) => md.push(format!(
            "**Instance** `{}` : `{component}`",
            ctx.instance_qualified_name
        )),
        None => md.push(format!("**Instance** `{}`", ctx.instance_qualified_name)),
    }

    md.push(String::new());
    match generated_targets(state, &ctx) {
        Ok(targets) => {
            md.push(
                "**Generated code** (Ctrl+click the `phase` keyword or the code to jump)"
                    .to_string(),
            );
            md.push(String::new());
            for t in &targets {
                let uri = crate::uri::from_file_path(&t.path).unwrap_or_default();
                let mut line = format!(
                    "- `{}` → [{}:{}]({uri}#L{})",
                    t.topology,
                    t.file_name(),
                    t.line + 1,
                    t.line + 1
                );
                if !t.exact {
                    line.push_str(" — snippet text not found, pointing at the phase section");
                }
                if t.stale {
                    line.push_str(" — **build cache may be stale** (generated file is older than this source; run `fprime-util build`)");
                }
                md.push(line);
            }
        }
        Err(Unresolved::UnknownPhase(_)) => {}
        Err(Unresolved::NoTopology) => {
            md.push("**Generated code**: instance is not part of any topology.".to_string());
        }
        Err(Unresolved::NoBuildCache) => {
            md.push(
                "**Generated code**: no build cache configured (set `buildCache` in `.fpp-lsp`)."
                    .to_string(),
            );
        }
        Err(Unresolved::NotGenerated(tops)) => {
            let tops: Vec<String> = tops.iter().map(|t| format!("`{t}`")).collect();
            md.push(format!(
                "**Generated code**: no `TopologyAc` file found in the build cache for {} — run `fprime-util generate`/`build`.",
                tops.join(", ")
            ));
        }
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: md.join("\n").trim().to_string(),
        }),
        range: Some(node_to_location(state, at.spec.id()).range),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATED: &str = r#"namespace Ref {

  Svc::WasmSequencer wasmSeq(FW_OPTIONAL_NAME("Ref.wasmSeq"));

}

namespace Ref {

  namespace ConfigObjects {

    namespace CdhCore_health {
      Svc::Health::PingEntry pingEntries[NUM_PING_ENTRIES] = {
        {
          PingEntries::CdhCore_cmdDisp::WARN,
          "CdhCore_cmdDisp"
        }
      };
    }

    namespace Ref_other {
      int x = 1;
    }

  }

  void initComponents(const TopologyState& state) {
    wasmSeq.init(QueueSizes::Ref_wasmSeq, InstanceIds::Ref_wasmSeq);
  }

  void configComponents(const TopologyState& state) {
    CdhCore::health.setPingEntries(
        ConfigObjects::CdhCore_health::pingEntries,
        FW_NUM_ARRAY_ELEMENTS(ConfigObjects::CdhCore_health::pingEntries),
        Svc::Health::HEALTH_WATCHDOG_CODE
    );
    wasmSeq.configure(Svc::WasmSequencer::Config(), memAllocator);
  }

  void regCommands() {
    wasmSeq.regCommands();
  }

}
"#;

    fn phase(name: &str) -> &'static PhaseInfo {
        PHASES.iter().find(|p| p.name == name).unwrap()
    }

    fn line_of(text: &str, needle: &str) -> u32 {
        text.lines().position(|l| l.contains(needle)).unwrap() as u32
    }

    #[test]
    fn finds_snippet_in_function() {
        let code = "\n  wasmSeq.configure(Svc::WasmSequencer::Config(), memAllocator);\n  ";
        let (line, exact) =
            locate_snippet(GENERATED, phase("configComponents"), "Ref_wasmSeq", code);
        assert!(exact);
        assert_eq!(line, line_of(GENERATED, "wasmSeq.configure("));
    }

    #[test]
    fn finds_multiline_snippet_with_different_indentation() {
        let code = "CdhCore::health.setPingEntries(\n    ConfigObjects::CdhCore_health::pingEntries,\n    FW_NUM_ARRAY_ELEMENTS(ConfigObjects::CdhCore_health::pingEntries),\n    Svc::Health::HEALTH_WATCHDOG_CODE\n);";
        let (line, exact) =
            locate_snippet(GENERATED, phase("configComponents"), "CdhCore_health", code);
        assert!(exact);
        assert_eq!(line, line_of(GENERATED, "CdhCore::health.setPingEntries("));
    }

    #[test]
    fn falls_back_to_function_when_snippet_missing() {
        let (line, exact) = locate_snippet(
            GENERATED,
            phase("configComponents"),
            "Ref_wasmSeq",
            "wasmSeq.configure(NewConfig());",
        );
        assert!(!exact);
        assert_eq!(line, line_of(GENERATED, "void configComponents("));
    }

    #[test]
    fn does_not_match_snippet_outside_its_phase_function() {
        // The same text exists in `regCommands`, not `initComponents`.
        let (line, exact) = locate_snippet(
            GENERATED,
            phase("initComponents"),
            "Ref_wasmSeq",
            "wasmSeq.regCommands();",
        );
        // Falls back to a whole-file search (the build cache may predate a
        // phase move) and still finds it.
        assert!(exact);
        assert_eq!(line, line_of(GENERATED, "wasmSeq.regCommands();"));
    }

    #[test]
    fn finds_config_object_namespace() {
        let code = "int x = 1;";
        let (line, exact) = locate_snippet(GENERATED, phase("configObjects"), "Ref_other", code);
        assert!(exact);
        assert_eq!(line, line_of(GENERATED, "int x = 1;"));

        let (line, exact) =
            locate_snippet(GENERATED, phase("configObjects"), "Ref_other", "int y;");
        assert!(!exact);
        assert_eq!(line, line_of(GENERATED, "namespace Ref_other {"));
    }

    #[test]
    fn finds_instance_definition() {
        let code = "Svc::WasmSequencer wasmSeq(FW_OPTIONAL_NAME(\"Ref.wasmSeq\"));";
        let (line, exact) = locate_snippet(GENERATED, phase("instances"), "Ref_wasmSeq", code);
        assert!(exact);
        assert_eq!(line, line_of(GENERATED, "Svc::WasmSequencer wasmSeq("));
    }

    #[test]
    fn missing_section_points_at_top() {
        let (line, exact) = locate_snippet(GENERATED, phase("startTasks"), "Ref_wasmSeq", "foo();");
        assert!(!exact);
        assert_eq!(line, 0);
    }

    #[test]
    fn unescapes_fpp_strings() {
        assert_eq!(unescape(r#"a \"b\" \\ c"#), r#"a "b" \ c"#);
        assert_eq!(unescape("plain"), "plain");
    }

    #[test]
    fn parses_fprime_locations_pairs() {
        let pairs = parse_fprime_locations(
            "/proj\n/proj/build-fprime-automatic-native\n/fprime\n/proj/build-fprime-automatic-native/F-Prime\n",
        );
        assert_eq!(pairs.len(), 2);
        assert_eq!(
            map_source_to_build(&pairs, Path::new("/proj/Ref/Top")),
            Some(PathBuf::from("/proj/build-fprime-automatic-native/Ref/Top"))
        );
        assert_eq!(
            map_source_to_build(&pairs, Path::new("/fprime/Svc/Top")),
            Some(PathBuf::from(
                "/proj/build-fprime-automatic-native/F-Prime/Svc/Top"
            ))
        );
        assert_eq!(map_source_to_build(&pairs, Path::new("/elsewhere")), None);
    }

    #[test]
    fn longest_source_root_wins() {
        let pairs = vec![
            (PathBuf::from("/a"), PathBuf::from("/b1")),
            (PathBuf::from("/a/lib"), PathBuf::from("/b2")),
        ];
        assert_eq!(
            map_source_to_build(&pairs, Path::new("/a/lib/x")),
            Some(PathBuf::from("/b2/x"))
        );
    }

    #[test]
    fn phase_table_matches_fpp_to_cpp_ordinals() {
        for (i, p) in PHASES.iter().enumerate() {
            assert_eq!(p.ordinal, i as i128, "{}", p.name);
        }
        assert_eq!(phase_info(4).unwrap().name, "configComponents");
        assert!(phase_info(13).is_none());
    }
}
