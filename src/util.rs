/// splitmix64: cheap deterministic hash for stable per-cell randomness.
pub fn hash(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Hash to a float in [0, 1).
pub fn hash01(x: u64) -> f32 {
    (hash(x) >> 40) as f32 / (1u64 << 24) as f32
}

const GLYPHS: &[char] = &[
    '░', '▒', '▓', '█', '╔', '╗', '╚', '╝', '═', '║', '╬', '<', '>', '/', '\\', '|', '+', '*', '#', '%', '@', '&', '$',
    '!', '?', '0', '1', 'A', 'B', 'C', 'D', 'E', 'F', 'x', 'k', 'z', '¥', '§', 'Σ', 'Ω', 'λ',
];

pub fn glyph(h: u64) -> char {
    GLYPHS[(hash(h) % GLYPHS.len() as u64) as usize]
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_quad(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t
}

pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub fn ease_out_expo(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * t) }
}

pub fn ease_out_sine(t: f32) -> f32 {
    (t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin()
}

pub fn ease_in_out_sine(t: f32) -> f32 {
    -((t.clamp(0.0, 1.0) * std::f32::consts::PI).cos() - 1.0) / 2.0
}

pub fn ease_in_expo(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 { 0.0 } else { 2f32.powf(10.0 * t - 10.0) }
}

pub fn ease_in_out_expo(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 || t >= 1.0 {
        t
    } else if t < 0.5 {
        2f32.powf(20.0 * t - 10.0) / 2.0
    } else {
        (2.0 - 2f32.powf(-20.0 * t + 10.0)) / 2.0
    }
}

/// Overshoots a little at both ends (values briefly leave 0..1).
pub fn ease_in_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c = 1.701_58 * 1.525;
    if t < 0.5 {
        (2.0 * t).powi(2) * ((c + 1.0) * 2.0 * t - c) / 2.0
    } else {
        f32::midpoint((2.0 * t - 2.0).powi(2) * ((c + 1.0) * (t * 2.0 - 2.0) + c), 2.0)
    }
}

pub fn ease_out_bounce(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d {
        n * t * t
    } else if t < 2.0 / d {
        let t = t - 1.5 / d;
        n * t * t + 0.75
    } else if t < 2.5 / d {
        let t = t - 2.25 / d;
        n * t * t + 0.9375
    } else {
        let t = t - 2.625 / d;
        n * t * t + 0.984_375
    }
}

/// Cells on the straight line from `a` to `b` (Bresenham), both included.
pub fn line(a: (i32, i32), b: (i32, i32)) -> Vec<(i32, i32)> {
    let (mut x, mut y) = a;
    let (dx, dy) = ((b.0 - x).abs(), -(b.1 - y).abs());
    let (sx, sy) = ((b.0 - x).signum(), (b.1 - y).signum());
    let mut err = dx + dy;
    let mut out = Vec::with_capacity((dx - dy) as usize + 1);
    loop {
        out.push((x, y));
        if (x, y) == b {
            return out;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}
