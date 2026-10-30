/**
 * Front end of the CLI: turn a set of FPP files into a sprotty `SGraph` using
 * the existing `fpp-diagram` binary (the same `fpp_diagram` lowering the
 * language server runs for the VS Code diagram panel).
 *
 * `fpp-diagram` reads a single translation unit, so the input files are
 * concatenated into one temporary file. `include "..."` directives are
 * resolved relative to the file that contains them and inlined, which keeps
 * the concatenated unit self-contained.
 */
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { spawnSync } from "child_process";
import { SGraph } from "sprotty-protocol";

const INCLUDE_RE = /^(\s*)include\s+"([^"]+)"\s*$/;

/** Inline `include` directives of a single FPP file, recursively. */
export function inlineIncludes(file: string, seen: Set<string> = new Set()): string {
    const resolved = path.resolve(file);
    if (seen.has(resolved)) {
        throw new Error(`include cycle at ${resolved}`);
    }
    seen.add(resolved);
    const dir = path.dirname(resolved);
    const lines = fs.readFileSync(resolved, "utf8").split(/\r?\n/);
    const out: string[] = [];
    for (const line of lines) {
        const m = INCLUDE_RE.exec(line);
        if (m) {
            out.push(inlineIncludes(path.join(dir, m[2]), seen));
        } else {
            out.push(line);
        }
    }
    seen.delete(resolved);
    return out.join("\n");
}

/** Concatenate FPP files into one translation unit with includes inlined. */
export function concatenateSources(files: string[]): string {
    return files.map(f => inlineIncludes(f)).join("\n\n");
}

/** Candidate locations for `fpp-diagram` when none is given explicitly. */
export function findFppDiagram(explicit?: string): string {
    if (explicit) {
        return explicit;
    }
    // A workspace build next to this extension (editors/code -> repo root).
    const repoRoot = path.resolve(__dirname, "..", "..", "..");
    for (const profile of ["release", "debug"]) {
        const candidate = path.join(repoRoot, "target", profile, "fpp-diagram");
        if (fs.existsSync(candidate)) {
            return candidate;
        }
    }
    return "fpp-diagram"; // rely on PATH
}

export interface LowerOptions {
    fppDiagram: string;
    kind: string;
    name: string;
    hideUnusedPorts: boolean;
}

/** Run `fpp-diagram` on the FPP files and parse its sprotty JSON. */
export function lowerToSGraph(files: string[], opts: LowerOptions): SGraph {
    const unit = concatenateSources(files);
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "fpp-diagram-render-"));
    const tmpFile = path.join(tmpDir, "unit.fpp");
    try {
        fs.writeFileSync(tmpFile, unit);
        const args = ["--kind", opts.kind, "--name", opts.name, "--format", "sprotty"];
        if (opts.hideUnusedPorts) {
            args.push("--hide-unused-ports");
        }
        args.push(tmpFile);
        const result = spawnSync(opts.fppDiagram, args, { encoding: "utf8", maxBuffer: 1 << 28 });
        if (result.error) {
            throw new Error(`failed to run ${opts.fppDiagram}: ${result.error.message}`);
        }
        if (result.status !== 0) {
            throw new Error(
                `${opts.fppDiagram} exited with ${result.status}\n${result.stderr}${result.stdout}`
            );
        }
        return JSON.parse(result.stdout) as SGraph;
    } finally {
        fs.rmSync(tmpDir, { recursive: true, force: true });
    }
}
