/**
 * Approximate text measurement for headless rendering.
 *
 * The webview measures label bounds with the browser's `getBBox()`; there is no
 * DOM in the CLI, so label sizes are estimated from Helvetica advance widths
 * (AFM metrics, 1/1000 em). The diagram CSS forces an 11 px sans-serif font,
 * which is what the generated SVG declares, so the estimate is close to what a
 * viewer draws.
 */

export const FONT_SIZE_PX = 11;
export const FONT_FAMILY = "Helvetica, Arial, sans-serif";

/** Advance widths for printable ASCII 0x20..0x7E, in 1/1000 em (Helvetica). */
const ADVANCES: number[] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // ' ' .. '/'
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // '0' .. '?'
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // '@' .. 'O'
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // 'P' .. '_'
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // '`' .. 'o'
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,      // 'p' .. '~'
];
const DEFAULT_ADVANCE = 556;

export function textWidth(text: string, fontSize: number = FONT_SIZE_PX): number {
    let units = 0;
    for (const ch of text) {
        const code = ch.codePointAt(0) ?? 0;
        const idx = code - 0x20;
        units += idx >= 0 && idx < ADVANCES.length ? ADVANCES[idx] : DEFAULT_ADVANCE;
    }
    return Math.ceil((units / 1000) * fontSize);
}

/** Line box for a single line of text (line-height ~1.2). */
export function textHeight(fontSize: number = FONT_SIZE_PX): number {
    return Math.ceil(fontSize * 1.2);
}

/** Baseline offset from the top of the line box (ascent ~0.9 em in a 1.2 line box). */
export function textBaseline(fontSize: number = FONT_SIZE_PX): number {
    return Math.round(fontSize * 0.95);
}
