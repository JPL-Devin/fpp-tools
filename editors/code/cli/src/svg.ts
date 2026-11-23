/**
 * Serialize a laid-out sprotty `SGraph` to standalone SVG.
 *
 * The markup mirrors the webview views (`webview/src/views.tsx`): component
 * nodes are rounded rectangles classed by component kind, collapsed topologies
 * are dashed boxes, boundary ports are chevrons, ports are small rectangles
 * classed by port kind, labels are `<text>` elements, and edges are polylines
 * with an arrow marker (dashed when implicit, fat with a slash when a bus).
 * The stylesheet is `webview/css/diagram.css` with the VS Code theme variables
 * resolved to fixed colors.
 */
import { SModelElement, SNode, SPort, SLabel, SEdge, SGraph, Point } from "sprotty-protocol";
import { BoundarySNode, ComponentSNode, FppSEdge, PortSNode, TopologySNode } from "../../common/models";
import { FONT_FAMILY, FONT_SIZE_PX, textBaseline } from "./text-metrics";

export type Theme = "dark" | "light";

interface Palette {
    background: string;
    nodeFill: string;
    nodeText: string;
    topologyFill: string;
    foreground: string;
    edge: string;
    boundaryInput: string;
    boundaryOutput: string;
}

const PALETTES: Record<Theme, Palette> = {
    // VS Code "Dark Modern" defaults for the variables diagram.css uses.
    dark: {
        background: "#1f1f1f", nodeFill: "#0078d4", nodeText: "#ffffff", topologyFill: "#3a3d41",
        foreground: "#cccccc", edge: "#cccccc", boundaryInput: "#3794ff", boundaryOutput: "#d18616",
    },
    // For documents: pale fills and dark text, legible on paper and on a white page.
    light: {
        background: "#ffffff", nodeFill: "#dbe9f9", nodeText: "#1b1b1b", topologyFill: "#f1f1f1",
        foreground: "#1b1b1b", edge: "#3b3b3b", boundaryInput: "#9cc8ff", boundaryOutput: "#f5c58a",
    },
};

function stylesheet(p: Palette): string {
    return `
    text { font-family: ${FONT_FAMILY}; font-size: ${FONT_SIZE_PX}px; font-weight: normal; }
    .sprotty-node { fill: ${p.nodeFill}; stroke: ${p.foreground}; stroke-width: 1.5; }
    .sprotty-label { fill: ${p.foreground}; stroke: none; }
    .node-label { fill: ${p.nodeText}; }
    .sprotty-port { fill: ${p.foreground}; stroke: #7b337d; stroke-width: 1; }
    .sprotty-edge { fill: none; stroke: ${p.edge}; }
    .sprotty-edge-arrow { fill: ${p.edge}; stroke-width: 0; }
    .component-active { stroke: #3FB984; }
    .component-passive { stroke: #fe92a8; }
    .topology-node { fill: ${p.topologyFill}; stroke: ${p.foreground}; stroke-dasharray: 8 4; }
    .boundary-port { stroke: ${p.foreground}; stroke-width: 1; }
    .boundary-input { fill: ${p.boundaryInput}; }
    .boundary-output { fill: ${p.boundaryOutput}; }
    .sprotty-edge path.edge-implicit { stroke-dasharray: 5 4; opacity: 0.85; }
    .sprotty-edge path.edge-bus { stroke-width: 3px; }
    .sprotty-edge path.edge-bus.edge-implicit { stroke-dasharray: 8 5; }
    .sprotty-edge .edge-bus-slash { stroke-width: 1.5px; }
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

function renderLabel(label: SLabel, inNode: boolean): string {
    const pos = label.position ?? { x: 0, y: 0 };
    const y = pos.y + textBaseline();
    const cls = inNode ? "sprotty-label node-label" : "sprotty-label";
    return `<text class="${cls}" x="${fmt(pos.x)}" y="${fmt(y)}">${esc(label.text)}</text>`;
}

function renderPort(port: SPort & Partial<PortSNode>): string {
    const pos = port.position ?? { x: 0, y: 0 };
    const size = port.size ?? { width: 10, height: 10 };
    const classes = ["sprotty-port"];
    if (port.kind === "sync") classes.push("port-sync");
    if (port.kind === "async") classes.push("port-async");
    if (port.kind === "guarded") classes.push("port-guarded");
    const children = (port.children ?? []).map(c => renderChild(c, false)).join("");
    return `<g id="${esc(port.id)}" transform="translate(${fmt(pos.x)},${fmt(pos.y)})">` +
        `<title>${esc(port.id)}${port.kind ? ` (${esc(port.kind)})` : ""}</title>` +
        `<rect class="${classes.join(" ")}" width="${fmt(size.width)}" height="${fmt(size.height)}"/>` +
        children + `</g>`;
}

function renderNode(node: SNode): string {
    const pos = node.position ?? { x: 0, y: 0 };
    const size = node.size ?? { width: 0, height: 0 };
    const w = size.width, h = size.height;
    const children = (node.children ?? []).map(c => renderChild(c, true)).join("");
    const open = `<g id="${esc(node.id)}" transform="translate(${fmt(pos.x)},${fmt(pos.y)})"`;
    if (node.type === "node:topology") {
        const name = (node as SNode & TopologySNode).qualifiedName;
        return `${open} data-topology="${esc(name)}"><title>Subtopology ${esc(name)}</title>` +
            `<rect class="sprotty-node topology-node" width="${fmt(w)}" height="${fmt(h)}" rx="4"/>` +
            children + `</g>`;
    }
    if (node.type === "node:boundary") {
        const tip = Math.min(10, h / 2);
        const chevron = [[0, 0], [w - tip, 0], [w, h / 2], [w - tip, h], [0, h], [tip, h / 2]]
            .map(([x, y]) => `${fmt(x)},${fmt(y)}`).join(" ");
        const dir = (node as SNode & BoundarySNode).isOutput ? "boundary-output" : "boundary-input";
        // The label is centred on the chevron whatever width layout gave it.
        const labels = (node.children ?? []).filter(c => basicType(c) === "label").map(c => {
            const label = c as SLabel;
            const y = (h - (label.size?.height ?? 0)) / 2 + textBaseline();
            return `<text class="sprotty-label node-label" text-anchor="middle" x="${fmt(w / 2)}" y="${fmt(y)}">` +
                `${esc(label.text)}</text>`;
        }).join("");
        return `${open}><polygon class="sprotty-node boundary-port ${dir}" points="${chevron}"/>` +
            labels + `</g>`;
    }
    const kind = (node as SNode & Partial<ComponentSNode>).kind;
    const classes = ["sprotty-node", "task"];
    if (kind === "active") classes.push("component-active");
    if (kind === "queued") classes.push("component-queued");
    if (kind === "passive") classes.push("component-passive");
    return `${open}><rect class="${classes.join(" ")}" width="${fmt(w)}" height="${fmt(h)}" rx="10"/>` +
        children + `</g>`;
}

/** The bus slash: a short stroke across the wire at its midpoint, 60 degrees off the wire. */
function busSlash(pts: Point[]): string {
    let total = 0;
    for (let i = 1; i < pts.length; i++) {
        total += Math.hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y);
    }
    if (total === 0) return "";
    let remaining = total / 2;
    for (let i = 1; i < pts.length; i++) {
        const a = pts[i - 1], b = pts[i];
        const length = Math.hypot(b.x - a.x, b.y - a.y);
        if (length < remaining && i < pts.length - 1) {
            remaining -= length;
            continue;
        }
        const t = length === 0 ? 0 : remaining / length;
        const mid = { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t };
        const dir = length === 0 ? { x: 1, y: 0 } : { x: (b.x - a.x) / length, y: (b.y - a.y) / length };
        const angle = Math.PI / 3, half = 7;
        const sx = dir.x * Math.cos(angle) - dir.y * Math.sin(angle);
        const sy = dir.x * Math.sin(angle) + dir.y * Math.cos(angle);
        return `<line class="edge-bus-slash" x1="${fmt(mid.x - sx * half)}" y1="${fmt(mid.y - sy * half)}" ` +
            `x2="${fmt(mid.x + sx * half)}" y2="${fmt(mid.y + sy * half)}"/>`;
    }
    return "";
}

function renderEdge(edge: SEdge & FppSEdge): string {
    const pts = edge.routingPoints ?? [];
    if (pts.length < 2) {
        return "";
    }
    const d = pts.map((p, i) => `${i === 0 ? "M" : "L"} ${fmt(p.x)},${fmt(p.y)}`).join(" ");
    const title = edge.detail ? `<title>${esc(edge.detail)}</title>` : "";
    const bus = (edge.count ?? 1) > 1;
    const classes = [edge.implicit === true ? "edge-implicit" : "", bus ? "edge-bus" : ""].filter(c => c);
    const cls = classes.length > 0 ? ` class="${classes.join(" ")}"` : "";
    const labels = (edge.children ?? []).map(c => renderChild(c, false)).join("");
    return `<g id="${esc(edge.id)}" class="sprotty-edge">${title}<path${cls} d="${d}" marker-end="url(#arrow)"/>` +
        (bus ? busSlash(pts) : "") + labels + `</g>`;
}

/** `inNode`: the element is a direct child of a node, so a label is drawn on the node's fill. */
function renderChild(e: SModelElement, inNode: boolean): string {
    switch (basicType(e)) {
        case "node": return renderNode(e as SNode);
        case "port": return renderPort(e as SPort);
        case "label": return renderLabel(e as SLabel, inNode);
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
    const body = (graph.children ?? []).map(c => renderChild(c, false)).join("\n");
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
