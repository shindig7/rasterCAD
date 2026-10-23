# rasterCAD

OpenSCAD-style scripts that render 2D raster images. Coordinates are in pixels, with the origin at the top-left and y pointing down.

```sh
cargo build --release
./target/release/rastercad examples/badges.rcad              # editor + live preview
./target/release/rastercad examples/badges.rcad -o out.png   # headless render
```

Editor keys: **F5** render, **Ctrl+S** save and render, **Ctrl+E** export PNG, **Ctrl+O** / **Ctrl+N** open / new. In the preview, scroll to zoom and drag to pan. The corner readout shows the canvas x, y and the color under the cursor. Click an error to jump to its line.

## Language

The full guide with examples is in [docs/GUIDE.md](docs/GUIDE.md). In the editor, press **F1** (or **Help → Language guide**) to open it.

The syntax and semantics follow OpenSCAD: variables, `function f(x) = ...;`, `module m(a, b = 2) { ... children(); }`, `children(i)`, `$children`, `for (i = [0 : 2 : 10])`, `if / else`, `let`, `a ? b : c`, list comprehensions (`[for (...) if (...) x]`, `each`), `.x` / `.y`, and `*stmt` to disable a statement. Angles are in degrees.

| Builtin | Notes |
|---|---|
| `canvas(width, height, bg)` | Required, once. `bg` defaults to transparent. |
| `rect(size, center=false, r=0)` | `size` is a number or `[w, h]`. `r` is the corner radius. |
| `circle(r)` / `circle(d=)`, `ellipse([rx, ry])` | Centered on the origin. |
| `polygon(points)`, `line(from, to, width=1)` | |
| `text(t, size=16, font="sans-serif", halign="left", valign="baseline", color, outline_thickness=0, outline_color="black")` | `font`: `"DejaVu Sans:style=Bold Italic"`, `serif`/`monospace`, or a `.ttf` path. Fonts in `~/.config/rasterCAD/fonts` work by family or file name. |
| `image(file, size=[w, h], center=false)` | PNG/JPEG/etc. If one entry of `size` is `undef`, the aspect ratio is kept. |
| `translate([x, y])`, `rotate(a, center=[x, y])`, `scale(s or [sx, sy])`, `mirror([x, y])` | Positive rotation is clockwise on screen. |
| `color(c, alpha)` | Any CSS color (`"steelblue"`, `"#ff000080"`, `"hsl(...)"`) or `[r, g, b, a]` in 0..1. |
| `stroke(width)` | Outlines child shapes (text included) instead of filling them. |
| `opacity(a)`, `blur(r)` | Layer effects. `blur` takes a standard deviation in canvas pixels. |
| `echo(...)`, `assert(cond, msg)` | |

Functions: `sin cos tan asin acos atan atan2 abs sqrt pow exp ln log floor ceil round sign min max norm in_to_px px_to_in len concat str chr ord is_undef is_num is_string is_list is_bool`, plus `PI`.

## Libraries

`use <lib.rcad>` imports a file's modules and functions; its variables stay private to it. `include <lib.rcad>` pastes the whole file in. Paths are searched in this order: the script's directory, then each directory in `$RASTERCAD_PATH`, then `~/.local/share/rastercad/libraries`. See `examples/lib/badges.rcad`.

## Adding a filter

Write `fn(&mut Pixmap, &[f64]) -> Result<(), String>` and add an entry to `FILTERS` in `src/render/filters.rs`. It then works as a module that applies to its children, e.g. `myfilter(1, 2) { ... }`, and the editor highlights it.
