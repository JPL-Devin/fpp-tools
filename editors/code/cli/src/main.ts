/**
 * `fpp-diagram-render`: command-line rendering of FPP diagrams to SVG.
 *
 * Pipeline (same stages as the VS Code diagram panel, minus the webview):
 *   1. `fpp-diagram --format sprotty` lowers the FPP model to a sprotty SGraph.
 *   2. Labels get sizes (the webview measures them in the DOM; here they are
 *      estimated from font metrics).
 *   3. `FppLayoutEngine` + `FppDiagramConfig` (the extension's ELK layout) place
 *      nodes, ports and labels.
 *   4. The laid-out graph is written as SVG using the webview's shapes and CSS.
 */
import * as fs from "fs";
import ELK from "elkjs/lib/elk.bundled.js";
import { SGraph, SLabel, SModelElement } from "sprotty-protocol";
import { FppLayoutEngine } from "../../src/diagram/layout";
import { FppDiagramConfig, DiagramType } from "../../src/diagram/layout-config";
import { findFppDiagram, lowerToSGraph } from "./fpp-source";
import { textHeight, textWidth } from "./text-metrics";
import { Theme, toSvg } from "./svg";

interface CliArgs {
    name?: string;
    kind: DiagramType;
    output?: string;
    json?: string;
    fppDiagram?: string;
    hideUnusedPorts: boolean;
    theme: Theme;
    files: string[];
}

const USAGE = `usage: fpp-diagram-render --name <qualified-name> [options] <file.fpp>...

Render an FPP component, topology, or connection group diagram to SVG using the
same lowering (fpp-diagram) and layout (ELK) as the VS Code extension.

options:
  --name <name>          fully qualified name, e.g. Svc.CommandDispatcher
                         (connection group: <topology>.<group>)
  --kind <kind>          component | topology | connectionGroup  [component]
  -o, --output <file>    SVG output path [stdout]
  --json <file>          read a sprotty SGraph JSON instead of running fpp-diagram
  --fpp-diagram <path>   fpp-diagram binary [target/{release,debug}/fpp-diagram, PATH]
  --hide-unused-ports    prune ports with no connections (topologies)
  --theme <dark|light>   color palette [dark]
  -h, --help             show this help

Multiple .fpp files may be given; \`include\` directives are resolved relative to
each file and inlined before analysis.`;

function parseArgs(argv: string[]): CliArgs {
    const args: CliArgs = { kind: DiagramType.component, hideUnusedPorts: false, theme: "dark", files: [] };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        const next = () => {
            if (i + 1 >= argv.length) throw new Error(`${a} requires a value`);
            return argv[++i];
        };
        switch (a) {
            case "--name": args.name = next(); break;
            case "--kind": {
                const k = next();
                if (!Object.values(DiagramType).includes(k as DiagramType) || k === DiagramType.stateMachine) {
                    throw new Error(`unsupported kind '${k}' (component | topology | connectionGroup)`);
                }
                args.kind = k as DiagramType;
                break;
            }
            case "-o": case "--output": args.output = next(); break;
            case "--json": args.json = next(); break;
            case "--fpp-diagram": args.fppDiagram = next(); break;
            case "--hide-unused-ports": args.hideUnusedPorts = true; break;
            case "--theme": {
                const t = next();
                if (t !== "dark" && t !== "light") throw new Error(`unknown theme '${t}'`);
                args.theme = t;
                break;
            }
            case "-h": case "--help": console.log(USAGE); process.exit(0);
            default:
                if (a.startsWith("-")) throw new Error(`unknown option '${a}'`);
                args.files.push(a);
        }
    }
    return args;
}

/** Give every label a size, as the webview's bounds round-trip would. */
function measureLabels(element: SModelElement): void {
    if (element.type.startsWith("label")) {
        const label = element as SLabel;
        label.size = { width: textWidth(label.text), height: textHeight() };
    }
    for (const child of element.children ?? []) {
        measureLabels(child);
    }
}

async function layout(graph: SGraph, kind: DiagramType, hideUnusedPorts: boolean): Promise<SGraph> {
    const config = new FppDiagramConfig();
    config.currentDiagramType = kind;
    config.hideUnusedPorts = hideUnusedPorts;
    const engine = new FppLayoutEngine(() => new ELK(), undefined, config);
    return engine.layout(graph);
}

export async function run(argv: string[]): Promise<void> {
    const args = parseArgs(argv);
    let graph: SGraph;
    if (args.json) {
        graph = JSON.parse(fs.readFileSync(args.json, "utf8")) as SGraph;
    } else {
        if (!args.name) throw new Error("--name is required");
        if (args.files.length === 0) throw new Error("at least one .fpp file is required");
        graph = lowerToSGraph(args.files, {
            fppDiagram: findFppDiagram(args.fppDiagram),
            kind: args.kind,
            name: args.name,
            hideUnusedPorts: args.hideUnusedPorts,
        });
    }
    measureLabels(graph);
    const laidOut = await layout(graph, args.kind, args.hideUnusedPorts);
    const svg = toSvg(laidOut, args.theme);
    if (args.output) {
        fs.writeFileSync(args.output, svg);
    } else {
        process.stdout.write(svg);
    }
}

run(process.argv.slice(2)).catch(e => {
    console.error(`fpp-diagram-render: ${e instanceof Error ? e.message : e}`);
    process.exit(1);
});
