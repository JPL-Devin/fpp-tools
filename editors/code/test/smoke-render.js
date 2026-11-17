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
        // Compact component diagram: one port per declaration, typed.
        const deployment = path.join(fixtures, "deployment.fpp");
        const compact = run(["--fpp-diagram", bin, "--name", "Rover.Mobility", "--compact-ports", deployment]);
        assert.ok(compact.includes(">motorOut: [6] Fw.Buffer</text>"), "compact port label");
        assert.ok(!compact.includes('id="Rover.Mobility.motorOut.1"'), "array drawn once");
        assert.ok(compact.includes(">timeCaller: Fw.Time</text>"), "special ports are drawn by default");
        const general = run(["--fpp-diagram", bin, "--name", "Rover.Mobility", "--compact-ports",
            "--hide-special-ports", deployment]);
        assert.ok(!general.includes("timeCaller"), "special ports are left out on request");
        assert.ok(general.includes(">motorOut: [6] Fw.Buffer</text>"), "general ports stay");

        // Collapsed topology of an FPP-only design: imported topologies are
        // single dashed nodes, parallel wires are a bus, implicit wires are dashed.
        const top = ["--fpp-diagram", bin, "--kind", "topology", "--name", "Rover.Rover"];
        const collapsed = run([...top, "--view", "collapsed", "--bundle-edges", "--hide-unused-ports", deployment]);
        assert.strictEqual((collapsed.match(/data-topology="/g) || []).length, 2, "two collapsed topologies");
        assert.ok(collapsed.includes('class="sprotty-node topology-node"'), "dashed topology box");
        assert.ok(!collapsed.includes('id="Comms.radio"'), "imported instances are hidden");
        assert.ok(/class="edge-implicit edge-bus"/.test(collapsed), "implicit bus edge");
        assert.ok(collapsed.includes('class="edge-bus-slash"'), "bus slash");
        assert.ok(collapsed.includes("timeCaller -&gt;"), "pattern connections are drawn by default");
        const plain = run([...top, "--view", "collapsed", "--hide-pattern-connections", deployment]);
        assert.ok(!plain.includes("timeCaller -&gt;"), "pattern connections are left out on request");
        const flat = run([...top, deployment]);
        assert.ok(flat.includes('id="Comms.radio"'), "flattened view is unchanged");
        assert.ok(!flat.includes("data-topology"), "flattened view has no topology nodes");

        // Boundary ports of a subtopology are chevrons with centred labels.
        const sub = run(["--fpp-diagram", bin, "--kind", "topology", "--name", "Comms.Subtopology",
            "--view", "collapsed", deployment]);
        assert.ok(/<polygon class="sprotty-node boundary-port boundary-output"/.test(sub), "output chevron");
        assert.ok(/text-anchor="middle"[^>]*>uplink</.test(sub), "boundary label");

        // Document set: one image per topology, a Markdown fragment, an interactive page.
        const docDir = fs.mkdtempSync(path.join(os.tmpdir(), "fpp-render-doc-"));
        const listed = run([...top, "--document", docDir, "--theme", "light", deployment]).trim().split("\n");
        const names = fs.readdirSync(docDir).sort();
        assert.deepStrictEqual(names, [
            "Cdh.Subtopology.svg", "Comms.Subtopology.svg",
            "Rover.Rover.html", "Rover.Rover.md", "Rover.Rover.svg",
        ]);
        assert.strictEqual(listed.length, names.length, "every written file is listed");
        const md = fs.readFileSync(path.join(docDir, "Rover.Rover.md"), "utf8");
        assert.ok(md.startsWith("![Topology Rover.Rover](Rover.Rover.svg)"), "top-level image first");
        assert.strictEqual((md.match(/<details>/g) || []).length, 2, "one details block per subtopology");
        assert.ok(md.includes("> ![Topology Comms.Subtopology](Comms.Subtopology.svg)"), "subtopology image");
        const page = fs.readFileSync(path.join(docDir, "Rover.Rover.html"), "utf8");
        assert.strictEqual((page.match(/<section data-name="/g) || []).length, 3, "one section per topology");
        assert.ok(!page.includes("<?xml"), "svg is inlined without the xml declaration");

        // Misuse is reported, not silently ignored.
        const bad = spawnSync(process.execPath, [cli, "--name", "Rover.Imager", "--document", docDir, deployment],
            { encoding: "utf8" });
        assert.notStrictEqual(bad.status, 0, "--document needs a topology");
        assert.ok(bad.stderr.includes("--document requires --kind topology"));
        const escape = spawnSync(process.execPath, [cli, "--kind", "topology", "--name", "../Rover",
            "--document", docDir, deployment], { encoding: "utf8" });
        assert.notStrictEqual(escape.status, 0, "--name cannot leave the output directory");
        assert.ok(escape.stderr.includes("is not a qualified name"));
        assert.deepStrictEqual(fs.readdirSync(docDir).sort(), names, "nothing written on misuse");
        console.log(`smoke-render: full pipeline OK (${bin})`);
    }
}
