/**
 * Serialize a laid-out sprotty `SGraph` to standalone SVG.
 *
 * The markup mirrors the webview views (`webview/src/views.tsx`): component
 * nodes are rounded rectangles classed by component kind, ports are small
 * rectangles classed by port kind, labels are `<text>` elements, and edges are
 * polylines with an arrow marker. The stylesheet is `webview/css/diagram.css`
 * with the VS Code theme variables resolved to fixed colors.
 */
import { SModelElement, SNode, SPort, SLabel, SEdge, SGraph, Point } from "sprotty-protocol";
import { FONT_FAMILY, FONT_SIZE_PX, textBaseline } from "./text-metrics";

export type Theme = "dark" | "light";

interface Palette {
    background: string;
    nodeFill: string;
    foreground: string;
    edge: string;
}

const PALETTES: Record<Theme, Palette> = {
    // VS Code "Dark Modern" defaults for the variables diagram.css uses.
    dark: { background: "#1f1f1f", nodeFill: "#0078d4", foreground: "#cccccc", edge: "#cccccc" },
    // VS Code "Light Modern" defaults.
    light: { background: "#ffffff", nodeFill: "#005fb8", foreground: "#3b3b3b", edge: "#171184" },
};

function stylesheet(p: Palette): string {
    return `
    text { font-family: ${FONT_FAMILY}; font-size: ${FONT_SIZE_PX}px; font-weight: normal; }
    .sprotty-node { fill: ${p.nodeFill}; stroke-width: 1; }
    .sprotty-label { fill: ${p.foreground}; }
    .sprotty-port { fill: ${p.foreground}; stroke: #7b337d; stroke-width: 1; }
    .sprotty-edge { fill: none; stroke: ${p.edge}; }
    .sprotty-edge-arrow { fill: ${p.edge}; stroke-width: 0; }
    .component-active { stroke: #3FB984; }
    .component-passive { stroke: #fe92a8; }
    .port-sync { fill: #fe92a8; }
    .port-guarded { fill: #FDD35C; }
    `;
}

function esc(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function fmt(n: number): string {
    return Number.isInteger(n) ? String(n) : n.toFixed(2);
}

function basicType(e: SModelElement): string {
    const i = e.type.indexOf(":");
    return i >= 0 ? e.type.substring(0, i) : e.type;
}

type Shaped = SModelElement & { position?: Point; size?: { width: number; height: number } };

function renderLabel(label: SLabel): string {
    const pos = label.position ?? { x: 0, y: 0 };
    const y = pos.y + textBaseline();
    return `<text class="sprotty-label" x="${fmt(pos.x)}" y="${fmt(y)}">${esc(label.text)}</text>`;
}

function renderPort(port: SPort & { kind?: string }): string {
    const pos = port.position ?? { x: 0, y: 0 };
    const size = port.size ?? { width: 10, height: 10 };
    const classes = ["sprotty-port"];
    if (port.kind === "sync") classes.push("port-sync");
    if (port.kind === "async") classes.push("port-async");
    if (port.kind === "guarded") classes.push("port-guarded");
    const children = (port.children ?? []).map(renderChild).join("");
    return `<g id="${esc(port.id)}" transform="translate(${fmt(pos.x)},${fmt(pos.y)})">` +
        `<title>${esc(port.id)}${port.kind ? ` (${esc(port.kind)})` : ""}</title>` +
        `<rect class="${classes.join(" ")}" width="${fmt(size.width)}" height="${fmt(size.height)}"/>` +
        children + `</g>`;
}

function renderNode(node: SNode & { kind?: string }): string {
    const pos = node.position ?? { x: 0, y: 0 };
    const size = node.size ?? { width: 0, height: 0 };
    const classes = ["sprotty-node", "task"];
    if (node.kind === "active") classes.push("component-active");
    if (node.kind === "queued") classes.push("component-queued");
    if (node.kind === "passive") classes.push("component-passive");
    const children = (node.children ?? []).map(renderChild).join("");
    return `<g id="${esc(node.id)}" transform="translate(${fmt(pos.x)},${fmt(pos.y)})">` +
        `<rect class="${classes.join(" ")}" width="${fmt(size.width)}" height="${fmt(size.height)}" rx="10"/>` +
        children + `</g>`;
}

function renderEdge(edge: SEdge & { detail?: string }): string {
    const pts = edge.routingPoints ?? [];
    if (pts.length < 2) {
        return "";
    }
    const d = pts.map((p, i) => `${i === 0 ? "M" : "L"} ${fmt(p.x)},${fmt(p.y)}`).join(" ");
    const title = edge.detail ? `<title>${esc(edge.detail)}</title>` : "";
    const labels = (edge.children ?? []).map(renderChild).join("");
    return `<g id="${esc(edge.id)}" class="sprotty-edge">${title}<path d="${d}" marker-end="url(#arrow)"/>${labels}</g>`;
}

function renderChild(e: SModelElement): string {
    switch (basicType(e)) {
        case "node": return renderNode(e as SNode);
        case "port": return renderPort(e as SPort);
        case "label": return renderLabel(e as SLabel);
        case "edge": return renderEdge(e as SEdge);
        default: return "";
    }
}

/**
 * Bounding box of everything drawn, for the viewBox. Port labels sit outside
 * their node, so nested shapes are walked with their accumulated offsets.
 */
function bounds(graph: SGraph): { x: number; y: number; width: number; height: number } {
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    const add = (x: number, y: number) => {
        minX = Math.min(minX, x); minY = Math.min(minY, y);
        maxX = Math.max(maxX, x); maxY = Math.max(maxY, y);
    };
    const walk = (e: SModelElement, dx: number, dy: number) => {
        if (basicType(e) === "edge") {
            for (const p of (e as SEdge).routingPoints ?? []) add(dx + p.x, dy + p.y);
            for (const c of e.children ?? []) walk(c, dx, dy);
            return;
        }
        const s = e as Shaped;
        const pos = s.position ?? { x: 0, y: 0 };
        const size = s.size ?? { width: 0, height: 0 };
        const x = dx + pos.x, y = dy + pos.y;
        add(x, y);
        add(x + size.width, y + size.height);
        for (const c of e.children ?? []) walk(c, x, y);
    };
    for (const c of graph.children ?? []) walk(c, 0, 0);
    if (!Number.isFinite(minX)) {
        return { x: 0, y: 0, width: 1, height: 1 };
    }
    return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}

export function toSvg(graph: SGraph, theme: Theme = "dark", padding: number = 20): string {
    const p = PALETTES[theme];
    const b = bounds(graph);
    const x = b.x - padding, y = b.y - padding;
    const w = b.width + 2 * padding, h = b.height + 2 * padding;
    const body = (graph.children ?? []).map(renderChild).join("\n");
    return `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="${fmt(w)}" height="${fmt(h)}" viewBox="${fmt(x)} ${fmt(y)} ${fmt(w)} ${fmt(h)}">
<style>${stylesheet(p)}</style>
<defs>
<marker id="arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse" class="sprotty-edge-arrow"><path d="M 0 0 L 10 5 L 0 10 z"/></marker>
</defs>
<rect x="${fmt(x)}" y="${fmt(y)}" width="${fmt(w)}" height="${fmt(h)}" fill="${p.background}"/>
${body}
</svg>
`;
}
