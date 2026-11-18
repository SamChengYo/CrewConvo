// The app icon, drawn in code so no image file has to ship with the source.
// Included by both the app (window and tray icon) and build.rs (the .exe icon).

fn icon_rounded(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> bool {
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let cx = x.clamp(x0 + r, x1 - r);
    let cy = y.clamp(y0 + r, y1 - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// 0 = outside, 1 = background, 2 = bubble. Coordinates are 0..1.
fn icon_layer(x: f32, y: f32) -> u8 {
    if !icon_rounded(x, y, 0.04, 0.04, 0.96, 0.96, 0.22) {
        return 0;
    }
    for cx in [0.36_f32, 0.50, 0.64] {
        if (x - cx).powi(2) + (y - 0.45).powi(2) <= 0.048 * 0.048 {
            return 1;
        }
    }
    if icon_rounded(x, y, 0.19, 0.25, 0.81, 0.65, 0.13) {
        return 2;
    }
    // The bubble's tail: a triangle below its lower-left corner.
    let (ax, ay, bx, by, cx, cy) = (0.30_f32, 0.64_f32, 0.46_f32, 0.64_f32, 0.29_f32, 0.80_f32);
    let side = |px: f32, py: f32, qx: f32, qy: f32| (x - px) * (qy - py) - (y - py) * (qx - px);
    let (d1, d2, d3) = (side(ax, ay, bx, by), side(bx, by, cx, cy), side(cx, cy, ax, ay));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    if !(neg && pos) { 2 } else { 1 }
}

/// RGBA pixels of the icon at `size` × `size`.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    const BG: [f32; 3] = [79.0, 70.0, 229.0];
    const FG: [f32; 3] = [255.0, 255.0, 255.0];
    const N: u32 = 4; // samples per pixel side
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut rgb, mut alpha) = ([0.0_f32; 3], 0.0_f32);
            for sy in 0..N {
                for sx in 0..N {
                    let x = (px as f32 + (sx as f32 + 0.5) / N as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / N as f32) / size as f32;
                    let color = match icon_layer(x, y) {
                        0 => continue,
                        1 => BG,
                        _ => FG,
                    };
                    alpha += 1.0;
                    for i in 0..3 {
                        rgb[i] += color[i];
                    }
                }
            }
            if alpha == 0.0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                for c in rgb {
                    out.push((c / alpha).round() as u8);
                }
                out.push((alpha / (N * N) as f32 * 255.0).round() as u8);
            }
        }
    }
    out
}
