// The ratiod icon, drawn by code (same design as web/public/favicon.svg): a green rounded
// square with a white up arrow. Also compiled into build.rs through `include!`, hence plain
// `//` comments and no crate imports.

const GREEN: [f32; 3] = [5.0, 150.0, 105.0];
const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
const SAMPLES: u32 = 4;

/// Distance from point `p` to the segment `a`-`b`.
fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
    let (dx, dy) = (p.0 - (a.0 + t * abx), p.1 - (a.1 + t * aby));
    (dx * dx + dy * dy).sqrt()
}

/// Whether the point (in a 32x32 coordinate space) is inside the rounded square (radius 8).
fn in_square(x: f32, y: f32) -> bool {
    let (dx, dy) = ((x - 16.0).abs() - 8.0, (y - 16.0).abs() - 8.0);
    let (ox, oy) = (dx.max(0.0), dy.max(0.0));
    (ox * ox + oy * oy).sqrt() <= 8.0
}

/// Whether the point (32x32 space) is on the arrow: a 3-wide stroke with round caps.
fn in_arrow(x: f32, y: f32) -> bool {
    let p = (x, y);
    [((16.0, 23.0), (16.0, 9.0)), ((16.0, 9.0), (10.0, 15.0)), ((16.0, 9.0), (22.0, 15.0))]
        .into_iter()
        .any(|(a, b)| segment_distance(p, a, b) <= 1.5)
}

/// `size` x `size` straight-alpha RGBA pixels. Pixels outside the rounded square are fully transparent.
pub fn rgba(size: u32) -> Vec<u8> {
    let scale = 32.0 / size as f32;
    let total = (SAMPLES * SAMPLES) as f32;
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut square, mut arrow) = (0u32, 0u32);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * scale;
                    if in_square(x, y) {
                        square += 1;
                        if in_arrow(x, y) {
                            arrow += 1;
                        }
                    }
                }
            }
            if square == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let white = arrow as f32 / square as f32;
            for channel in 0..3 {
                out.push((GREEN[channel] + (WHITE[channel] - GREEN[channel]) * white).round() as u8);
            }
            out.push((square as f32 / total * 255.0).round() as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::rgba;

    fn pixel(data: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [data[i], data[i + 1], data[i + 2], data[i + 3]]
    }

    #[test]
    fn has_the_expected_length() {
        assert_eq!(rgba(64).len(), 64 * 64 * 4);
        assert_eq!(rgba(16).len(), 16 * 16 * 4);
    }

    #[test]
    fn corners_are_transparent() {
        let data = rgba(64);
        for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63)] {
            assert_eq!(pixel(&data, 64, x, y)[3], 0);
        }
    }

    #[test]
    fn arrow_stem_is_white() {
        let data = rgba(64);
        let [r, g, b, a] = pixel(&data, 64, 32, 32);
        assert!(r > 240 && g > 240 && b > 240 && a == 255, "{r} {g} {b} {a}");
    }

    #[test]
    fn square_is_green_away_from_the_arrow() {
        let data = rgba(64);
        assert_eq!(pixel(&data, 64, 3, 32), [0x05, 0x96, 0x69, 255]);
        assert_eq!(pixel(&data, 64, 32, 60), [0x05, 0x96, 0x69, 255]);
    }
}
