//! Raster filters. Each one is a script module like `blur(r) { ... }`: its children render to a
//! layer the size of the canvas, and `apply` runs on that layer. To add a filter, write a function
//! and add a `FilterDef` entry to `FILTERS`.

use tiny_skia::Pixmap;

#[derive(Debug)]
pub struct FilterDef {
    pub name: &'static str,
    /// Numeric parameters in positional order, with optional defaults.
    pub params: &'static [(&'static str, Option<f64>)],
    pub apply: fn(&mut Pixmap, &[f64]) -> Result<(), String>,
}

pub static FILTERS: &[FilterDef] = &[FilterDef {
    name: "blur",
    params: &[("r", None)],
    apply: blur,
}];

pub fn find(name: &str) -> Option<&'static FilterDef> {
    FILTERS.iter().find(|f| f.name == name)
}

/// Gaussian blur with standard deviation `r` canvas pixels, approximated by three box blurs.
/// Pixels outside the layer count as transparent.
fn blur(pixmap: &mut Pixmap, args: &[f64]) -> Result<(), String> {
    let sigma = args[0];
    if !(0.0..=1000.0).contains(&sigma) {
        return Err(format!(
            "blur radius must be between 0 and 1000, got {sigma}"
        ));
    }
    if sigma < 0.1 {
        return Ok(());
    }
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    // Premultiplied RGBA blurs correctly as-is. Work in f32 so the passes don't compound rounding.
    let data = pixmap.data_mut();
    let mut buf: Vec<f32> = data.iter().map(|&b| f32::from(b)).collect();
    let mut tmp = vec![0.0; buf.len()];
    for radius in box_radii(sigma) {
        box_pass(&buf, &mut tmp, w, h, radius, 4, w * 4); // horizontal
        box_pass(&tmp, &mut buf, h, w, radius, w * 4, 4); // vertical (transposed indexing)
    }
    for (d, v) in data.iter_mut().zip(&buf) {
        *d = v.round().clamp(0.0, 255.0) as u8;
    }
    Ok(())
}

/// Radii of three successive box blurs that approximate a Gaussian of deviation `sigma`.
fn box_radii(sigma: f64) -> [usize; 3] {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut lower = ideal.floor() as usize;
    if lower.is_multiple_of(2) {
        lower -= 1;
    }
    let upper = lower + 2;
    let lf = lower as f64;
    let m = ((12.0 * sigma * sigma - 3.0 * lf * lf - 12.0 * lf - 9.0) / (-4.0 * lf - 4.0)).round();
    std::array::from_fn(|i| if (i as f64) < m { lower / 2 } else { upper / 2 })
}

/// One box blur along lines. `step` moves along a line, `line_stride` between lines.
fn box_pass(
    src: &[f32],
    dst: &mut [f32],
    line_len: usize,
    lines: usize,
    radius: usize,
    step: usize,
    line_stride: usize,
) {
    let window = (2 * radius + 1) as f32;
    for line in 0..lines {
        let base = line * line_stride;
        for ch in 0..4 {
            let at = |i: usize| src[base + i * step + ch];
            let mut sum: f32 = (0..=radius.min(line_len - 1)).map(at).sum();
            for i in 0..line_len {
                dst[base + i * step + ch] = sum / window;
                if i + radius + 1 < line_len {
                    sum += at(i + radius + 1);
                }
                if i >= radius {
                    sum -= at(i - radius);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_spreads_and_preserves_mass() {
        let n = 61;
        let mut pm = Pixmap::new(n, n).unwrap();
        for y in 26..35 {
            for x in 26..35 {
                let i = ((y * n + x) * 4) as usize;
                pm.data_mut()[i..i + 4].copy_from_slice(&[255; 4]);
            }
        }
        blur(&mut pm, &[2.0]).unwrap();
        let alpha = |x: u32, y: u32| u32::from(pm.data()[((y * n + x) * 4 + 3) as usize]);
        assert!(alpha(30, 30) > 240, "center stays nearly opaque");
        assert!(alpha(26, 30) < 200, "edge softens");
        assert!(alpha(23, 30) > 0, "blur spreads outward");
        // Away from the canvas edges a blur only redistributes alpha.
        let total: u32 = (0..n * n).map(|i| alpha(i % n, i / n)).sum();
        let expected = 81 * 255;
        assert!(total.abs_diff(expected) < expected / 50, "total {total}");
    }
}
