// Smoke test for the `fpp-diagram-render` CLI bundle (dist/fpp-diagram-render.js).
//
// Always: lay out a canned sprotty SGraph (`--json`) and check the SVG has the
// component box, every port, and port labels placed on the expected side.
// When an `fpp-diagram` binary is available (workspace build or PATH): also run
// the full pipeline on a small FPP model.

const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");

const here = __dirname;
const cli = path.join(here, "..", "dist", "fpp-diagram-render.js");
const fixtures = path.join(here, "fixtures");

function run(args) {
    const result = spawnSync(process.execPath, [cli, ...args], { encoding: "utf8" });
    assert.strictEqual(result.status, 0, `cli failed: ${result.stderr}`);
    return result.stdout;
}

function translateX(svg, id) {
    const m = new RegExp(`<g id="${id.replace(/\./g, "\\.")}" transform="translate\\((-?[0-9.]+),`).exec(svg);
    assert.ok(m, `missing element ${id}`);
    return Number(m[1]);
}

// 1. Layout + SVG from a canned SGraph.
{
    const svg = run(["--json", path.join(fixtures, "component.json")]);
    assert.ok(svg.startsWith("<?xml"), "svg header");
    assert.ok(/<rect class="sprotty-node task component-active"/.test(svg), "active component box");
    for (const id of ["Demo.Widget.cmdIn.0", "Demo.Widget.tlmIn.0", "Demo.Widget.tlmOut.0"]) {
        assert.ok(svg.includes(`id="${id}"`), `port ${id}`);
    }
    assert.ok(/class="sprotty-port port-guarded"/.test(svg), "guarded port styling");
    assert.ok(svg.includes(">tlmOut</text>"), "port label text");
    // Input ports sit on the west edge (x <= 0 within the node), outputs on the east.
    const widgetWidth = Number(/<rect class="sprotty-node[^"]*" width="([0-9.]+)"/.exec(svg)[1]);
    assert.ok(translateX(svg, "Demo.Widget.cmdIn.0") <= 0, "input port on west side");
    assert.ok(translateX(svg, "Demo.Widget.tlmOut.0") >= widgetWidth - 10, "output port on east side");
    // Light theme swaps the palette.
    const light = run(["--json", path.join(fixtures, "component.json"), "--theme", "light"]);
    assert.ok(light.includes('fill="#ffffff"'), "light background");
    console.log("smoke-render: layout + svg OK");
}

// 2. Full pipeline through fpp-diagram, when a binary is available.
{
    const repoRoot = path.resolve(here, "..", "..", "..");
    const candidates = [
        path.join(repoRoot, "target", "release", "fpp-diagram"),
        path.join(repoRoot, "target", "debug", "fpp-diagram"),
    ];
    let bin = candidates.find(p => fs.existsSync(p));
    if (!bin) {
        const which = spawnSync("fpp-diagram", ["--version"], { encoding: "utf8" });
        if (!which.error && which.status === 0) {
            bin = "fpp-diagram";
        }
    }
    if (!bin) {
        console.log("smoke-render: fpp-diagram not found, skipping full pipeline");
    } else {
        const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "fpp-render-smoke-")), "widget.svg");
        run(["--fpp-diagram", bin, "--name", "Demo.Widget", "-o", out, path.join(fixtures, "component.fpp")]);
        const svg = fs.readFileSync(out, "utf8");
        assert.ok(svg.includes('id="Demo.Widget"'), "component node");
        assert.ok(svg.includes('id="Demo.Widget.logOut.1"'), "array port index 1");
        assert.ok(/class="sprotty-port port-sync"/.test(svg), "sync port styling");
        console.log(`smoke-render: full pipeline OK (${bin})`);
    }
}
