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

pub fn ease_in_cubic(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}

pub fn ease_in_quart(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(4)
}

pub fn ease_in_out_quad(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        2.0 * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
    }
}

pub fn ease_in_out_quart(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        8.0 * t.powi(4)
    } else {
        1.0 - (-2.0 * t + 2.0).powi(4) / 2.0
    }
}

pub fn ease_in_out_circ(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        (1.0 - (1.0 - (2.0 * t).powi(2)).sqrt()) / 2.0
    } else {
        (1.0 - (-2.0 * t + 2.0).powi(2)).sqrt().midpoint(1.0)
    }
}

/// When `ease` (increasing from 0 to 1) reaches `y`: the time a sequence
/// paced by it gets to item `y` of 0..1.
pub fn ease_inverse(ease: fn(f32) -> f32, y: f32) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = lo.midpoint(hi);
        if ease(mid) < y { lo = mid } else { hi = mid }
    }
    hi
}

pub fn ease_out_expo(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t >= 1.0 { 1.0 } else { 1.0 - 2f32.powf(-10.0 * t) }
}

pub fn ease_out_circ(t: f32) -> f32 {
    (1.0 - (t.clamp(0.0, 1.0) - 1.0).powi(2)).sqrt()
}

pub fn ease_out_quint(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(5)
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
