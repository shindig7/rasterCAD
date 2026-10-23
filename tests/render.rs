//! End-to-end tests: script source in, pixels out.

use std::path::{Path, PathBuf};

use rastercad::{Pixmap, render};

fn ok(src: &str) -> Pixmap {
    ok_at(src, None)
}

fn ok_at(src: &str, path: Option<&Path>) -> Pixmap {
    match render(src, path).image {
        Ok(p) => p,
        Err(d) => panic!("render failed: {d}"),
    }
}

fn err(src: &str) -> rastercad::Diagnostic {
    render(src, None).image.expect_err("expected an error")
}

/// Un-premultiplied RGBA at (x, y).
fn px(p: &Pixmap, x: u32, y: u32) -> [u8; 4] {
    let c = p.pixel(x, y).expect("in bounds").demultiply();
    [c.red(), c.green(), c.blue(), c.alpha()]
}

const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A fresh, empty directory for file-based tests.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rastercad-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn rect_at_position() {
    let p = ok(r#"canvas(50, 50, bg="white"); color("red") translate([10, 10]) rect([20, 20]);"#);
    assert_eq!(px(&p, 15, 15), RED);
    assert_eq!(px(&p, 5, 5), WHITE);
    assert_eq!(px(&p, 31, 15), WHITE);
}

#[test]
fn rotation_is_clockwise_in_y_down_space() {
    // A 20x4 bar rotated 90° points down from the pivot, extending to the left of it.
    let p = ok(r#"canvas(50, 50, bg="white"); translate([25, 25]) rotate(90) rect([20, 4]);"#);
    assert_eq!(px(&p, 23, 35), BLACK);
    assert_eq!(px(&p, 35, 27), WHITE);
}

#[test]
fn scale_mirror_and_center() {
    let p = ok(r#"canvas(40, 40, bg="white");
        translate([20, 20]) mirror([1, 0]) scale(2) rect([5, 5]);"#);
    // Mirrored across the vertical axis: the 10x10 square sits left of x = 20.
    assert_eq!(px(&p, 15, 25), BLACK);
    assert_eq!(px(&p, 25, 25), WHITE);
    let p = ok(r#"canvas(40, 40, bg="white"); translate([20, 20]) rect(10, center=true);"#);
    assert_eq!(px(&p, 16, 16), BLACK);
    assert_eq!(px(&p, 26, 26), WHITE);
}

#[test]
fn modules_children_and_loops() {
    let p = ok(r#"
        canvas(100, 20, bg="white");
        module row(gap = 30) {
            for (i = [0 : $children - 1]) translate([i * gap, 0]) children(i);
        }
        row() { rect(10); color("red") rect(10); circle(5); }
    "#);
    assert_eq!(px(&p, 5, 5), BLACK);
    assert_eq!(px(&p, 35, 5), RED);
    assert_eq!(px(&p, 60, 0), BLACK); // circle centered on the origin of the 3rd slot
    assert_eq!(px(&p, 15, 5), WHITE);
}

#[test]
fn functions_comprehensions_and_echo() {
    let out = render(
        r#"
        canvas(1, 1);
        function sq(x) = x * x;
        squares = [for (i = [1:3]) sq(i)];
        echo(squares, total = squares[0] + squares.y + squares[2], s = str("n=", len(squares)));
        echo(let(a = 2) a ^ 3, max([4, 9, 2]), [for (x = [1:6]) if (x % 2 == 0) x]);
    "#,
        None,
    );
    assert!(out.image.is_ok());
    assert_eq!(
        out.log,
        vec![
            r#"ECHO: [1, 4, 9], total = 14, s = "n=3""#,
            "ECHO: 8, 9, [2, 4, 6]",
        ]
    );
}

#[test]
fn stroke_opacity_and_blur() {
    let p = ok(r#"canvas(60, 60, bg="white");
        stroke(2) translate([10, 10]) rect([40, 40]);
        opacity(0.5) translate([20, 20]) rect([20, 20]);"#);
    assert_eq!(px(&p, 10, 30), BLACK); // on the outline
    assert_eq!(px(&p, 15, 15), WHITE); // inside the outline, unfilled
    let gray = px(&p, 30, 30)[0];
    assert!(
        (120..=135).contains(&gray),
        "half-transparent black over white, got {gray}"
    );

    let p = ok(r#"canvas(60, 60, bg="white"); blur(3) translate([20, 20]) rect([20, 20]);"#);
    let edge = px(&p, 20, 30)[0];
    assert!(
        (60..=200).contains(&edge),
        "blurred edge is gray, got {edge}"
    );
    assert_eq!(px(&p, 30, 30), BLACK);
}

#[test]
fn text_renders_glyphs() {
    let p = ok(r#"canvas(120, 60, bg="white");
        translate([60, 30]) text("Hi", size=40, halign="center", valign="center");"#);
    let dark = (0..120)
        .flat_map(|x| (0..60).map(move |y| (x, y)))
        .filter(|&(x, y)| px(&p, x, y)[0] < 128)
        .collect::<Vec<_>>();
    assert!(dark.len() > 100, "expected glyph pixels");
    // Centered: ink exists on both sides of the middle, and none at the far edges.
    assert!(dark.iter().any(|&(x, _)| x < 60) && dark.iter().any(|&(x, _)| x > 60));
    assert!(dark.iter().all(|&(x, _)| x > 5 && x < 115));

    // Fill and outline colors: a thick outline surrounds red letters.
    let p = ok(r#"canvas(120, 60, bg="white");
        translate([60, 30]) text("I", size=40, halign="center", valign="center",
            color="red", outline_thickness=4, outline_color="blue");"#);
    let row: Vec<[u8; 4]> = (0..120).map(|x| px(&p, x, 30)).collect();
    let first = |c: [u8; 4]| row.iter().position(|&p| p == c);
    let (blue, red) = (first([0, 0, 255, 255]), first(RED));
    assert!(
        blue.is_some() && red.is_some(),
        "expected blue outline and red fill"
    );
    assert!(blue < red, "outline sits outside the fill");
    // `{-}` draws an em dash.
    let dash = |t: &str| {
        ok(&format!(
            r#"canvas(80, 40, bg="white"); translate([5, 30]) text("{t}", size=20);"#
        ))
    };
    assert_eq!(dash("a{-}b").data(), dash("a—b").data());
    assert_ne!(dash("a{-}b").data(), dash("a-b").data());

    assert!(
        err(r#"canvas(9, 9); text("x", outline_thickness=-1);"#)
            .msg
            .contains("must not be negative")
    );
    assert!(
        err(r#"canvas(9, 9); text("x", font="No Such Font 123");"#)
            .msg
            .contains("not found")
    );
}

#[test]
fn libraries_use_and_include() {
    let dir = temp_dir("libs");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/shapes.rcad"),
        r#"
        size = 10;
        function double(x) = x * 2;
        module square_at(p) translate(p) rect(double(size / 2));
        rect(999); // top-level geometry is ignored by `use`
    "#,
    )
    .unwrap();
    std::fs::write(dir.join("consts.rcad"), "accent = \"red\";").unwrap();
    let main = dir.join("main.rcad");
    let src = r#"
        use <lib/shapes.rcad>
        include <consts.rcad>
        canvas(40, 40, bg="white");
        color(accent) square_at([20, 20]);
    "#;
    let p = ok_at(src, Some(&main));
    assert_eq!(px(&p, 25, 25), RED);
    assert_eq!(px(&p, 5, 5), WHITE);

    // Variables of a used library are private to it.
    let e = render(
        "use <lib/shapes.rcad>\ncanvas(9, 9); echo(size);",
        Some(&main),
    );
    assert!(e.image.unwrap_err().msg.contains("unknown variable 'size'"));
}

#[test]
fn include_cycle_is_reported() {
    let dir = temp_dir("cycle");
    std::fs::write(dir.join("a.rcad"), "include <b.rcad>").unwrap();
    std::fs::write(dir.join("b.rcad"), "include <a.rcad>").unwrap();
    let out = render(
        "include <a.rcad>\ncanvas(9, 9);",
        Some(&dir.join("main.rcad")),
    );
    assert!(out.image.unwrap_err().msg.contains("circular include"));
}

#[test]
fn image_import_scales_and_keeps_aspect() {
    let dir = temp_dir("image");
    let mut img = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]));
    img.put_pixel(0, 0, image::Rgba([0, 0, 255, 255]));
    img.save(dir.join("pic.png")).unwrap();
    let p = ok_at(
        r#"canvas(40, 40, bg="white"); image("pic.png", size=[20, undef]);"#,
        Some(&dir.join("main.rcad")),
    );
    // 4x2 scaled to 20x10.
    assert_eq!(px(&p, 15, 7)[..3], RED[..3]);
    assert_eq!(px(&p, 15, 15), WHITE);
}

#[test]
fn errors_point_at_the_problem() {
    let d = err("canvas(10, 10);\ncircle(r=\"x\");");
    assert_eq!((d.line, d.col), (2, 8));
    assert_eq!(d.msg, "circle(): 'r' must be a finite number, got string");

    assert!(
        err("module f() f();\ncanvas(9, 9);\nf();")
            .msg
            .contains("recursion too deep")
    );
    assert!(err("circle(5);").msg.contains("never calls canvas"));
    assert!(
        err("canvas(9, 9); foo();")
            .msg
            .contains("unknown module 'foo'")
    );
    assert!(
        err("canvas(9, 9); x = [1, 2] + [1];")
            .msg
            .contains("different lengths")
    );
    assert!(
        err("canvas(9, 9); rect(5) circle(1);")
            .msg
            .contains("does not take children")
    );
    assert!(
        err("canvas(9, 9); children();")
            .msg
            .contains("inside a module")
    );
    assert!(
        err("canvas(9, 9); color(\"nope\") rect(1);")
            .msg
            .contains("invalid color")
    );
}

#[test]
fn deep_but_finite_recursion_works() {
    let out = render(
        "canvas(1, 1); function sum(n) = n == 0 ? 0 : n + sum(n - 1); echo(sum(3000));",
        None,
    );
    assert!(out.image.is_ok(), "{:?}", out.image.err());
    assert_eq!(out.log, vec!["ECHO: 4501500"]);
}
