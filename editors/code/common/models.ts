/**
 * These models store minimal FPP information pertinent to rendering components.
 *
 * They match the extra fields emitted by the Rust `fpp_diagram` sprotty lowering
 * (`kind` on component nodes, `kind`/`isOutput` on ports) and are shared between
 * the extension host and the webview.
 */
import type { SEdge, SNode, SPort } from "sprotty-protocol";

export interface ComponentSNode extends SNode {
    kind: string
}

/** A declared port of the diagrammed topology, drawn on the diagram boundary. */
export interface BoundarySNode extends SNode {
    kind: string,
    isOutput: boolean, // Outputs are pinned to the last ELK layer, inputs to the first.
}

/** A collapsed imported topology; activating it opens that topology's diagram. */
export interface TopologySNode extends SNode {
    qualifiedName: string
}

export interface FppSEdge extends SEdge {
    implicit?: boolean, // Ends on a collapsed topology's boundary rather than a rendered port.
    detail?: string,    // Hover text.
    count?: number,     // Wires the edge stands for; more than one makes it a bus. Absent means 1.
}

export interface PortSNode extends SPort {
    kind: string,
    isOutput: boolean, // Store output info here for ELK layout config. Input ports are positioned west, and output ports are positioned east.
}
