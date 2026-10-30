# fpp-diagram-render

Command-line rendering of FPP diagrams to SVG, using the same pipeline as the
VS Code extension's diagram panel:

1. `fpp-diagram --format sprotty` (the Rust `fpp_diagram` lowering the language
   server uses) produces a sprotty `SGraph`.
2. Labels are sized from font metrics (the webview measures them in the DOM).
3. The extension's ELK layout (`src/diagram/layout.ts`,
   `src/diagram/layout-config.ts`) places nodes, ports and labels.
4. The laid-out graph is written as standalone SVG with the webview's shapes
   and stylesheet (`webview/css/diagram.css`, theme variables resolved).

## Build

```sh
cargo build --release -p fpp_diagram --bin fpp-diagram   # repo root
cd editors/code && yarn install && yarn build            # emits dist/fpp-diagram-render.js
```

## Use

```sh
# One component from the F Prime framework sources
node dist/fpp-diagram-render.js --name Svc.CommandDispatcher -o CommandDispatcher.svg \
    $FPRIME/cmake/platform/unix/Platform/PlatformTypes.fpp \
    $(find $FPRIME/Fpp $FPRIME/Fw $FPRIME/Svc $FPRIME/Drv $FPRIME/Os $FPRIME/Utils $FPRIME/default/config -name '*.fpp')

# A topology, pruning unconnected ports
node dist/fpp-diagram-render.js --kind topology --name Ref.Ref --hide-unused-ports -o Ref.svg <files...>

# PNG, if wanted (any SVG rasterizer works; headless Chrome shown)
google-chrome --headless=new --screenshot=CommandDispatcher.png --window-size=400,1100 CommandDispatcher.svg
```

Multiple `.fpp` files may be given; `include "..."` directives are resolved
relative to each file and inlined before analysis, so a whole source tree can be
passed. The `fpp-diagram` binary is found in `target/{release,debug}` of the
workspace or on `PATH`; override with `--fpp-diagram`.

`--kind` accepts `component` (default), `topology`, and `connectionGroup`
(`--name <topology>.<group>`). State machines are not rendered by this tool;
`fpp-diagram --format mermaid` already emits them as Mermaid text.
