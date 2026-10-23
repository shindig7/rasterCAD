# rasterCAD language guide

## Basics

A script describes an image. Statements end with `;`. Comments are `// line` or `/* block */`.

```
canvas(400, 200, bg="white");                 // required, exactly once
color("steelblue") translate([20, 20]) rect([160, 80], r=12);
translate([200, 120]) text("Hello", size=40, halign="center");
```

- Coordinates are pixels. The origin `[0, 0]` is the top-left corner, x grows right, y grows down.
- Shapes are drawn in order: later statements paint on top of earlier ones.
- A transform or style applies to the statement (or `{ block }`) right after it.
- Shapes are black and filled unless wrapped in `color()` or `stroke()`.
- Prefix a statement with `*` to disable it: `*circle(10);`

## Values

- Numbers: `12`, `-3.5`, `1e3`
- Strings: `"text"`, with escapes `\n \t \" \\ é`
- Booleans: `true`, `false`, and `undef` for "no value"
- Lists: `[1, 2, 3]`, points are lists `[x, y]`. Index with `p[0]`, or `p.x` / `p.y`
- Ranges: `[0 : 10]` (0 to 10 inclusive), `[0 : 2 : 10]` (step 2)
- `PI` is predefined

## Operators

From lowest to highest precedence:

| Operator | Meaning |
|---|---|
| `c ? a : b` | Conditional |
| `\|\|`  `&&`  `!` | Logical or, and, not |
| `==  !=  <  <=  >  >=` | Comparison |
| `+  -` | Add, subtract (also element-wise on lists) |
| `*  /  %` | Multiply, divide, remainder. List * number scales; list * list is the dot product |
| `^` | Power, right-associative: `2^3^2` is `2^9` |

Joining strings with `+` is an error; use `str("n = ", n)`.

## Variables

```
w = 120;
gap = w / 4;
```

Within a block, all assignments run before any drawing, and a variable has one value per block: if it's assigned twice, the last value is used everywhere in that block (as in OpenSCAD). Using an unknown variable is an error.

## Canvas

| Call | Notes |
|---|---|
| `canvas(width, height, bg)` | Image size in pixels (1 to 16384). `bg` is any color; transparent if omitted. |

For print work, size things in inches with `in_to_px`:

```
ppi = 300;
canvas(in_to_px(8.5, ppi), in_to_px(11, ppi), bg="white");   // US Letter at 300 PPI
translate(in_to_px([1, 1], ppi)) rect(in_to_px([2, 0.5], ppi));
```

## Shapes

| Call | Notes |
|---|---|
| `rect(size, center=false, r=0)` | `size` is a number (square) or `[w, h]`. Top-left at the origin unless `center=true`. `r` rounds the corners. |
| `circle(r)` or `circle(d=...)` | Centered on the origin. |
| `ellipse([rx, ry])` | Centered on the origin. A single number makes a circle. |
| `polygon(points)` | List of `[x, y]` points, at least 3. Closed automatically. |
| `line(from, to, width=1)` | Straight segment between two points. |

```
color("tomato") polygon([[0, 0], [60, 0], [30, 50]]);
line([0, 0], [100, 40], width=3);
```

## Text

| Call | Notes |
|---|---|
| `text(t, size=16, font="sans-serif", halign="left", valign="baseline", color, outline_thickness=0, outline_color="black")` | Draws `t` with its anchor at the origin. |

- `{-}` in the text draws an em dash (—)
- `halign`: `"left"`, `"center"`, `"right"`
- `valign`: `"top"`, `"center"`, `"baseline"`, `"bottom"`
- `font`: `"sans-serif"`, `"serif"`, `"monospace"`, an installed family like `"DejaVu Sans"`, or a `.ttf`/`.otf` file name or path
- Your own fonts: put `.ttf`/`.otf`/`.ttc` files in `~/.config/rasterCAD/fonts`. Use them by family name or file name from any script. In the editor, Insert → Font lists them and inserts `font="..."` at the cursor
- A font file name is looked for next to the script first, then in `~/.config/rasterCAD/fonts`
- Font styles go after a colon: `"DejaVu Sans:style=Bold Italic"`. Styles: thin, light, regular, medium, semibold, bold, extrabold, black, italic, oblique
- `color` sets the letter color. Without it, the text uses the surrounding `color()` (black by default)
- `outline_thickness` draws an outline that many pixels wide around the letters, in `outline_color`. `0` means no outline
- Run `fc-list : family` in a terminal to list installed fonts
- Text is drawn as outlines, so it stays sharp when scaled or rotated, and `stroke()` outlines it

```
translate([200, 50]) text("Title", size=36, font="serif:style=Bold", halign="center", valign="top");
translate([200, 150]) text("SALE", size=48, color="yellow", outline_thickness=3, outline_color="black");
```

## Images

| Call | Notes |
|---|---|
| `image(file, size, center=false)` | PNG, JPEG and other common formats. Path is relative to the script. |

`size` is `[w, h]` in pixels. Use `undef` for one side to keep the aspect ratio: `image("logo.png", size=[200, undef])`. Without `size`, the image is drawn at its natural size.

In the editor, Insert → Image… picks a file and inserts an `image("...");` line with its full path at the cursor.

## Transforms

| Call | Notes |
|---|---|
| `translate([x, y])` | Move. |
| `rotate(a, center=[x, y])` | Rotate by `a` degrees, clockwise on screen, around `center` (default the origin). |
| `scale(s)` or `scale([sx, sy])` | Scale around the origin. |
| `mirror([x, y])` | Reflect across the line through the origin perpendicular to `[x, y]`. `mirror([1, 0])` flips left-right. |

Transforms nest and apply from the inside out, as in OpenSCAD:

```
translate([100, 100]) rotate(45) rect(40, center=true);   // spin in place, then move
```

## Style

| Call | Notes |
|---|---|
| `color(c, alpha)` | `c` is a CSS color: `"red"`, `"#ff8800"`, `"#ff880080"`, `"rgb(255 0 0)"`, `"hsl(200 80% 50%)"`, or a list `[r, g, b]` / `[r, g, b, a]` with values 0 to 1. `alpha` (0 to 1) overrides the color's alpha. |
| `stroke(width)` | Draw outlines of the child shapes instead of filling them. |
| `opacity(a)` | Fade the whole group (0 to 1). Overlapping children don't show through each other. |

```
color("navy") stroke(2) circle(30);
opacity(0.5) { rect(40); translate([20, 20]) rect(40); }
```

## Filters

Filters render their children to a separate layer, process it, and draw the result.

| Call | Notes |
|---|---|
| `blur(r)` | Gaussian blur, `r` is the standard deviation in canvas pixels. |

```
// Drop shadow
color("black", 0.4) blur(4) translate([4, 4]) rect([100, 60], r=8);
color("white") rect([100, 60], r=8);
```

## Control flow

```
for (i = [0 : 4]) translate([i * 30, 0]) circle(10);
for (p = [[10, 10], [50, 20]]) translate(p) rect(8);
for (x = [0 : 2], y = [0 : 2]) translate([x * 20, y * 20]) rect(15);   // nested

if (big) circle(50); else circle(10);

let (r = 20, d = r * 2) rect(d);
```

## Functions

```
function double(x) = x * 2;
function clamp(x, lo = 0, hi = 1) = min(max(x, lo), hi);
function fact(n) = n <= 1 ? 1 : n * fact(n - 1);
```

Call with positional or named arguments: `clamp(v, hi = 10)`.

Builtin functions (angles in degrees):

| Function | Notes |
|---|---|
| `sin cos tan asin acos atan atan2(y, x)` | Trigonometry |
| `abs sqrt pow(b, e) exp ln log floor ceil round sign` | Math. `log` is base 10 |
| `min max` | Several numbers or one list |
| `norm(v)` | Length of a vector |
| `in_to_px(inches, ppi)` / `px_to_in(pixels, ppi)` | Convert between inches and pixels at a PPI. Also works on lists: `in_to_px([8.5, 11], 300)` |
| `len(x)` | Length of a list, string or range |
| `concat(a, b, ...)` | Join lists |
| `str(...)` | Convert and join into a string |
| `chr(n)` / `ord(c)` | Character code conversion |
| `is_undef is_num is_string is_list is_bool` | Type checks |

## List comprehensions

```
squares = [for (i = [1 : 5]) i * i];               // [1, 4, 9, 16, 25]
evens = [for (i = [1 : 10]) if (i % 2 == 0) i];
star = [for (i = [0 : 9]) let (r = i % 2 == 0 ? 50 : 20)
            [r * sin(i * 36), -r * cos(i * 36)]];
flat = [for (row = [[1, 2], [3]]) each row];          // [1, 2, 3]
polygon(star);
```

## Modules

Modules are reusable drawing blocks with parameters.

```
module badge(label, w = 120, fill = "teal") {
    color(fill) rect([w, 32], r=8);
    color("white") translate([w / 2, 16])
        text(label, size=14, halign="center", valign="center");
}

translate([10, 10]) badge("NEW");
translate([10, 50]) badge("SALE", fill = "crimson");
```

A module can wrap other shapes with `children()`:

| Call | Notes |
|---|---|
| `children()` | Draw all children passed to the module. |
| `children(i)` | Draw only child `i` (a number, list or range). |
| `$children` | Number of children passed. |

```
module row(gap = 40) {
    for (i = [0 : $children - 1]) translate([i * gap, 0]) children(i);
}

row() { circle(10); rect(20); color("red") circle(10); }
```

## Libraries

Put modules and functions in a separate `.rcad` file and import them.

| Statement | Notes |
|---|---|
| `use <file.rcad>` | Imports the file's modules and functions. Its variables and drawing are ignored. |
| `include <file.rcad>` | Pastes the whole file in, as if you'd typed it there. |

Files are searched in order:

1. Relative to the script's folder
2. Each folder in the `RASTERCAD_PATH` environment variable (separated by `:`)
3. `~/.local/share/rastercad/libraries`

Put your templates in `~/.local/share/rastercad/libraries` to use them from any script.

## Debugging

| Call | Notes |
|---|---|
| `echo(...)` | Print values to the messages panel: `echo(w, total = w * 2)`. |
| `assert(condition, message)` | Stop with an error if `condition` is false. |

Errors show the file, line and column. Click an error in the messages panel to jump to it. In the preview, the bottom-right corner shows the pixel position and color under the mouse, which helps with positioning.

## Editor keys

| Key | Action |
|---|---|
| `F1` | Show or hide this guide |
| `F5` | Render |
| `Ctrl+S` | Save and render |
| `Ctrl+Shift+S` | Save as |
| `Ctrl+O` / `Ctrl+N` | Open / new |
| `Ctrl+E` | Export PNG |
| Mouse wheel / drag | Zoom / pan the preview |
| Insert → Image… | Pick an image and insert an `image()` line at the cursor |
| Insert → Font | Insert `font="..."` for a font from `~/.config/rasterCAD/fonts` |
