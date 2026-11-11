//! Big-text fonts for the clock face.

/// Marks a cell that belongs to no input character (gaps between glyphs).
pub const NO_OWNER: usize = usize::MAX;

/// A rendered block of text: `w * h` chars, row-major.
pub struct Glyphs {
    pub w: usize,
    pub h: usize,
    pub chars: Vec<char>,
    /// For each cell, the index of the input character it was drawn from.
    pub owner: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum FontKind {
    /// Pick the biggest font that fits.
    Auto,
    /// figlet "ANSI Shadow" style, 6 rows.
    Shadow,
    /// Half-block pixel font, scaled up to fill the screen.
    Block,
    /// Plain text.
    Plain,
}

// ANSI Shadow digits. Each row of a glyph has the same width.
const SHADOW: [[&str; 6]; 16] = [
    [
        " ██████╗ ",
        "██╔═████╗",
        "██║██╔██║",
        "████╔╝██║",
        "╚██████╔╝",
        " ╚═════╝ ",
    ],
    [" ██╗", "███║", "╚██║", " ██║", " ██║", " ╚═╝"],
    ["██████╗ ", "╚════██╗", " █████╔╝", "██╔═══╝ ", "███████╗", "╚══════╝"],
    ["██████╗ ", "╚════██╗", " █████╔╝", " ╚═══██╗", "██████╔╝", "╚═════╝ "],
    ["██╗  ██╗", "██║  ██║", "███████║", "╚════██║", "     ██║", "     ╚═╝"],
    ["███████╗", "██╔════╝", "███████╗", "╚════██║", "███████║", "╚══════╝"],
    [
        " ██████╗ ",
        "██╔════╝ ",
        "███████╗ ",
        "██╔═══██╗",
        "╚██████╔╝",
        " ╚═════╝ ",
    ],
    ["███████╗", "╚════██║", "    ██╔╝", "   ██╔╝ ", "   ██║  ", "   ╚═╝  "],
    [" █████╗ ", "██╔══██╗", "╚█████╔╝", "██╔══██╗", "╚█████╔╝", " ╚════╝ "],
    [" █████╗ ", "██╔══██╗", "╚██████║", " ╚═══██║", " █████╔╝", " ╚════╝ "],
    ["   ", "██╗", "╚═╝", "██╗", "╚═╝", "   "],
    ["   ", "   ", "   ", "   ", "██╗", "╚═╝"],
    // Unit letters for days / hours / minutes (`2d 03:04:05`, `1h 05m`), and a space.
    ["██████╗ ", "██╔══██╗", "██║  ██║", "██║  ██║", "██████╔╝", "╚═════╝ "],
    ["██╗  ██╗", "██║  ██║", "███████║", "██╔══██║", "██║  ██║", "╚═╝  ╚═╝"],
    [
        "███╗   ███╗",
        "████╗ ████║",
        "██╔████╔██║",
        "██║╚██╔╝██║",
        "██║ ╚═╝ ██║",
        "╚═╝     ╚═╝",
    ],
    ["  ", "  ", "  ", "  ", "  ", "  "],
];
/// Digits are padded to this width so the clock doesn't jitter as they change.
const SHADOW_DIGIT_W: usize = 9;

/// 3x5 pixel bitmap for the block font (digits, clock punctuation and, for
/// titles, A-Z plus a little punctuation). Glyphs are 1-3 pixels wide.
#[rustfmt::skip]
fn pixels(c: char) -> Option<[&'static str; 5]> {
    Some(match c.to_ascii_uppercase() {
        '0' => ["###", "#.#", "#.#", "#.#", "###"],
        '1' => ["##.", ".#.", ".#.", ".#.", "###"],
        '2' => ["###", "..#", "###", "#..", "###"],
        '3' => ["###", "..#", ".##", "..#", "###"],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "###", "..#", "###"],
        '6' => ["###", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", "..#", "..#", "..#"],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "###"],
        ':' => [".", "#", ".", "#", "."],
        '.' => [".", ".", ".", ".", "#"],
        'A' => [".#.", "#.#", "###", "#.#", "#.#"],
        'B' => ["##.", "#.#", "##.", "#.#", "##."],
        'C' => [".##", "#..", "#..", "#..", ".##"],
        'D' => ["##.", "#.#", "#.#", "#.#", "##."],
        'E' => ["###", "#..", "##.", "#..", "###"],
        'F' => ["###", "#..", "##.", "#..", "#.."],
        'G' => [".##", "#..", "#.#", "#.#", ".##"],
        'H' => ["#.#", "#.#", "###", "#.#", "#.#"],
        'I' => ["###", ".#.", ".#.", ".#.", "###"],
        'J' => ["..#", "..#", "..#", "#.#", ".#."],
        'K' => ["#.#", "#.#", "##.", "#.#", "#.#"],
        'L' => ["#..", "#..", "#..", "#..", "###"],
        'M' => ["#.#", "###", "###", "#.#", "#.#"],
        'N' => ["##.", "#.#", "#.#", "#.#", "#.#"],
        'O' => [".#.", "#.#", "#.#", "#.#", ".#."],
        'P' => ["##.", "#.#", "##.", "#..", "#.."],
        'Q' => [".#.", "#.#", "#.#", "##.", ".##"],
        'R' => ["##.", "#.#", "##.", "#.#", "#.#"],
        'S' => [".##", "#..", ".#.", "..#", "##."],
        'T' => ["###", ".#.", ".#.", ".#.", ".#."],
        'U' => ["#.#", "#.#", "#.#", "#.#", "###"],
        'V' => ["#.#", "#.#", "#.#", "#.#", ".#."],
        'W' => ["#.#", "#.#", "###", "###", "#.#"],
        'X' => ["#.#", "#.#", ".#.", "#.#", "#.#"],
        'Y' => ["#.#", "#.#", ".#.", ".#.", ".#."],
        'Z' => ["###", "..#", ".#.", "#..", "###"],
        ' ' => ["..", "..", "..", "..", ".."],
        '-' => ["...", "...", "###", "...", "..."],
        '+' => ["...", ".#.", "###", ".#.", "..."],
        '/' => ["..#", "..#", ".#.", "#..", "#.."],
        '!' => ["#", "#", "#", ".", "#"],
        '?' => ["###", "..#", ".#.", "...", ".#."],
        ',' => [".", ".", ".", "#", "#"],
        '\'' => ["#", "#", ".", ".", "."],
        _ => return None,
    })
}

/// True if every char of `text` is in the shadow font.
pub fn shadow_supports(text: &str) -> bool {
    text.chars().all(|c| glyph_index(c).is_some())
}

/// True if every char of `text` can be drawn with the block font.
pub fn block_supports(text: &str) -> bool {
    text.chars().all(|c| pixels(c).is_some())
}

fn glyph_index(c: char) -> Option<usize> {
    match c {
        '0'..='9' => Some(c as usize - '0' as usize),
        ':' => Some(10),
        '.' => Some(11),
        'd' | 'D' => Some(12),
        'h' | 'H' => Some(13),
        'm' | 'M' => Some(14),
        ' ' => Some(15),
        _ => None,
    }
}

pub fn shadow(text: &str) -> Glyphs {
    let mut rows: Vec<Vec<(char, usize)>> = vec![Vec::new(); 6];
    for (n, c) in text.chars().enumerate() {
        let Some(g) = glyph_index(c) else { continue };
        let gw = SHADOW[g][0].chars().count();
        let pad_w = if g < 10 { SHADOW_DIGIT_W } else { gw };
        let left = (pad_w - gw) / 2;
        for (r, row) in rows.iter_mut().enumerate() {
            if n > 0 {
                row.push((' ', NO_OWNER));
            }
            row.extend(std::iter::repeat_n((' ', n), left));
            row.extend(SHADOW[g][r].chars().map(|ch| (ch, n)));
            row.extend(std::iter::repeat_n((' ', n), pad_w - gw - left));
        }
    }
    from_rows(&rows)
}

/// Width in columns of `text` in the block font at the given scale.
pub fn block_width(text: &str, scale: usize) -> usize {
    let px: usize =
        text.chars().filter_map(pixels).map(|g| g[0].len()).sum::<usize>() + text.chars().count().saturating_sub(1);
    px * scale
}

pub fn block_height(scale: usize) -> usize {
    (5 * scale).div_ceil(2)
}

pub fn block(text: &str, scale: usize) -> Glyphs {
    let scale = scale.max(1);
    // Build the pixel bitmap with 1px gaps between glyphs.
    let mut px: Vec<Vec<bool>> = vec![Vec::new(); 5];
    let mut px_owner = Vec::new();
    for (n, c) in text.chars().enumerate() {
        let Some(g) = pixels(c) else { continue };
        if n > 0 {
            px_owner.push(NO_OWNER);
        }
        px_owner.extend(std::iter::repeat_n(n, g[0].len()));
        for (r, row) in px.iter_mut().enumerate() {
            if n > 0 {
                row.push(false);
            }
            row.extend(g[r].chars().map(|ch| ch == '#'));
        }
    }
    let pw = px[0].len() * scale;
    let ph = 5 * scale;
    let get = |x: usize, y: usize| y < ph && px[y / scale][x / scale];
    let h = block_height(scale);
    let mut chars = Vec::with_capacity(pw * h);
    let mut owner = Vec::with_capacity(pw * h);
    for r in 0..h {
        for x in 0..pw {
            chars.push(match (get(x, 2 * r), get(x, 2 * r + 1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
            owner.push(px_owner[x / scale]);
        }
    }
    Glyphs { w: pw, h, chars, owner }
}

/// One line of text. Double-width chars take two cells (the second is a
/// placeholder); zero-width and control chars are dropped.
pub fn plain(text: &str) -> Glyphs {
    use unicode_width::UnicodeWidthChar;
    let mut row = Vec::new();
    for (i, c) in text.chars().enumerate() {
        match c.width() {
            Some(1) => row.push((c, i)),
            Some(2) => row.extend([(c, i), (crate::term::WIDE_TAIL, i)]),
            _ => {}
        }
    }
    from_rows(&[row])
}

fn from_rows(rows: &[Vec<(char, usize)>]) -> Glyphs {
    let w = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut chars = Vec::with_capacity(w * rows.len());
    let mut owner = Vec::with_capacity(w * rows.len());
    for r in rows {
        chars.extend(r.iter().map(|&(c, _)| c));
        owner.extend(r.iter().map(|&(_, o)| o));
        chars.extend(std::iter::repeat_n(' ', w - r.len()));
        owner.extend(std::iter::repeat_n(NO_OWNER, w - r.len()));
    }
    Glyphs {
        w,
        h: rows.len(),
        chars,
        owner,
    }
}

/// Stack `bottom` under `top` with `gap` blank rows, both centered. Cells
/// from `bottom` get no owner (they never take part in the tick scramble).
/// Returns the combined block and the first row of `bottom`.
pub fn stack(top: &Glyphs, gap: usize, bottom: &Glyphs) -> (Glyphs, usize) {
    let w = top.w.max(bottom.w);
    let h = top.h + gap + bottom.h;
    let mut chars = vec![' '; w * h];
    let mut owner = vec![NO_OWNER; w * h];
    let tx = (w - top.w) / 2;
    for i in 0..top.chars.len() {
        let (r, c) = (i / top.w, i % top.w);
        chars[r * w + tx + c] = top.chars[i];
        owner[r * w + tx + c] = top.owner[i];
    }
    let (bx, by) = ((w - bottom.w) / 2, top.h + gap);
    for i in 0..bottom.chars.len() {
        let (r, c) = (i / bottom.w, i % bottom.w);
        chars[(by + r) * w + bx + c] = bottom.chars[i];
    }
    (Glyphs { w, h, chars, owner }, by)
}

/// Render `text` with `kind`, falling back to smaller fonts if it won't fit
/// in `max_w` x `max_h`.
/// With `scale`, the block font is used at exactly that size if it fits.
pub fn render(text: &str, kind: FontKind, max_w: usize, max_h: usize, scale: Option<usize>) -> Glyphs {
    let fits = |g: &Glyphs| g.w <= max_w && g.h <= max_h;
    if let Some(s) = scale {
        let g = block(text, s);
        if fits(&g) {
            return g;
        }
    }
    // Text the chosen font can't draw (e.g. a word for --done-text) goes to the
    // block font, which has letters, or else plain text.
    let kind = match kind {
        FontKind::Shadow | FontKind::Auto if !shadow_supports(text) => FontKind::Block,
        FontKind::Block if !block_supports(text) => FontKind::Plain,
        k => k,
    };
    match kind {
        FontKind::Plain => plain(text),
        FontKind::Shadow => {
            let g = shadow(text);
            if fits(&g) {
                g
            } else {
                render(text, FontKind::Auto, max_w, max_h, None)
            }
        }
        FontKind::Block => {
            // Fill a good chunk of the screen, like the Omarchy screensaver logo.
            let mut s = 1;
            while block_width(text, s + 1) <= max_w * 4 / 5 && block_height(s + 1) <= max_h * 3 / 5 {
                s += 1;
            }
            let g = block(text, s);
            if fits(&g) { g } else { plain(text) }
        }
        FontKind::Auto => {
            let g = shadow(text);
            if fits(&g) {
                return g;
            }
            let g = block(text, 1);
            if fits(&g) { g } else { plain(text) }
        }
    }
}
