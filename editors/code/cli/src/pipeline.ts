/** The render pipeline shared by single-diagram and document-set output. */
import ELK from "elkjs/lib/elk.bundled.js";
import { SGraph, SLabel, SModelElement } from "sprotty-protocol";
import { FppLayoutEngine } from "../../src/diagram/layout";
import { FppDiagramConfig, DiagramType } from "../../src/diagram/layout-config";
import { textHeight, textWidth } from "./text-metrics";
import { Theme, toSvg } from "./svg";

/** Room for the two chevron points of a boundary port, which layout sizes to its label. */
const BOUNDARY_LABEL_MARGIN = 28;

/** Give every label a size, as the webview's bounds round-trip would. */
function measureLabels(element: SModelElement, margin: number = 0): void {
    if (element.type.startsWith("label")) {
        const label = element as SLabel;
        label.size = { width: textWidth(label.text) + margin, height: textHeight() };
    }
    for (const child of element.children ?? []) {
        measureLabels(child, element.type === "node:boundary" ? BOUNDARY_LABEL_MARGIN : 0);
    }
}

export interface LayoutOptions {
    kind: DiagramType;
    hideUnusedPorts: boolean;
    collapsed: boolean;
    bundleEdges: boolean;
    theme: Theme;
}

/** Measure, lay out and serialize a lowered graph. The graph is modified in place. */
export async function graphToSvg(graph: SGraph, opts: LayoutOptions): Promise<string> {
    measureLabels(graph);
    const config = new FppDiagramConfig();
    config.currentDiagramType = opts.kind;
    config.hideUnusedPorts = opts.hideUnusedPorts;
    config.collapseSubtopologies = opts.collapsed;
    config.bundleEdges = opts.bundleEdges;
    const engine = new FppLayoutEngine(() => new ELK(), undefined, config);
    return toSvg(await engine.layout(graph), opts.theme);
}
