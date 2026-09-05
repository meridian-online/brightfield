//! The Meridian prime mark, drawn from the design system's own path data.
//!
//! # Why this is not a PNG in `assets/`
//!
//! `meridian-design`'s `brand/` directory carries terms of its own — the code
//! around it is MIT and the artwork is reserved, and one of the things the
//! reserved terms forbid is redrawing or re-proportioning the mark. A
//! rasterised copy committed here would be a second artefact to keep in step
//! with the first, and the first is a Cargo dependency this crate already
//! takes. So the geometry comes from
//! [`meridian_design::brand::mark_path`] at run time rather than from a copy:
//! change the mark in the design system, rebuild, and this draws the new one.
//!
//! # Why it is rasterised here rather than tessellated
//!
//! The mark is six closed subpaths — the teeth — and epaint fills a closed
//! path by the same routine it fills a convex polygon by, which is exact only
//! while every subpath is convex. That is a property of the current export
//! rather than of the mark, and an export that stopped satisfying it would
//! degrade into visible artefacts on the front door rather than into a
//! failure anything catches. A scanline fill over the flattened outline has
//! no such precondition: [`Mark::mask`] is an even-odd coverage raster, so a
//! tooth that becomes concave, or an export that nests one subpath inside
//! another, draws correctly with this module unchanged.
//!
//! The raster is an **alpha mask**, built once at a fixed `MASK_SIZE` and tinted at
//! draw time. One texture therefore serves both modes and every size the mark
//! is drawn at, and the ink is a semantic token read at the call site rather
//! than a colour baked into a picture.

use std::sync::OnceLock;

/// The side of the square coverage mask, in texels.
///
/// The mark is drawn at 22 points in the title band and at heading size on
/// the front door, so 256 is comfortably above the largest raster any display
/// scaling asks for and the mask is downsampled rather than magnified.
const MASK_SIZE: usize = 256;

/// Samples per texel per axis when filling the mask.
///
/// The coverage of a texel is the fraction of its `SUPERSAMPLE` × `SUPERSAMPLE`
/// grid that lands inside the outline, so an edge crossing a texel diagonally
/// resolves to a value between 0 and 255 rather than to a hard step.
const SUPERSAMPLE: usize = 4;

/// How many line segments each cubic in the source path is flattened into.
///
/// The teeth are drawn with long, low-curvature cubics — the caps are the
/// tightest of them — and at [`MASK_SIZE`] a cap subtends well under a
/// hundred texels, so sixteen segments put the flattening error below the
/// supersampled sample spacing.
const CUBIC_SEGMENTS: usize = 16;

/// The mark as a set of closed polygons in a unit square, y down.
///
/// Built once from the design system's path and reused for the life of the
/// process. Holding polygons rather than the source string means
/// [`Mark::mask`] and any later consumer share one flattening.
pub struct Mark {
    /// One entry per closed subpath, each a ring of points in `0.0..=1.0`.
    subpaths: Vec<Vec<[f32; 2]>>,
}

/// The mark, parsed and flattened once.
///
/// `None` says the design system's SVG has lost its `d` attribute, or carries
/// a path command this parser does not implement. Both are build-time facts
/// about a vendored file rather than a circumstance a user reaches, and
/// `the_shipped_mark_parses` is what turns either into a failing test instead
/// of a missing picture. A caller handed `None` draws no mark.
pub fn mark() -> Option<&'static Mark> {
    static MARK: OnceLock<Option<Mark>> = OnceLock::new();
    MARK.get_or_init(|| {
        let path = meridian_design::brand::mark_path()?;
        Mark::parse(path, meridian_design::brand::MARK_VIEWBOX)
    })
    .as_ref()
}

impl Mark {
    /// Parse and flatten an SVG path over a square `viewbox`.
    ///
    /// Implements the commands the shipped export actually uses — `M`, `m`,
    /// `l`, `c` and `Z` — plus their absolute counterparts `L` and `C`. An
    /// unimplemented command returns `None` rather than a guess: an export
    /// that starts emitting arcs is a real change to the artwork and should
    /// stop this rather than be approximated. `the_shipped_mark_parses` is
    /// what reddens when the shipped path stops being one this understands.
    fn parse(path: &str, viewbox: f32) -> Option<Self> {
        let mut lexer = Lexer::new(path);
        let mut subpaths: Vec<Vec<[f32; 2]>> = Vec::new();
        let mut current: Vec<[f32; 2]> = Vec::new();
        // The point a `Z` returns to, which is what a following relative
        // `m` is measured from — not the last point drawn.
        let mut start = [0.0f32, 0.0];
        let mut at = [0.0f32, 0.0];
        let mut command = None;

        while let Some(token) = lexer.peek_command() {
            let (op, explicit) = match token {
                // A run of coordinates with no letter in front of it repeats
                // the previous command, which is how this export writes its
                // consecutive line segments.
                //
                // Closepath is the one command with no implicit-repeat form:
                // it takes no coordinates, so a bare pair after it advances
                // nothing and the loop re-derives the same `Z` for ever. The
                // SVG grammar agrees — a coordinate may not follow closepath
                // without a command letter between them — so this refuses
                // rather than guessing. `a_bare_pair_after_closepath_is_refused`
                // is what holds it.
                None => match command? {
                    'Z' | 'z' => return None,
                    repeated => (repeated, false),
                },
                Some(c) => (c, true),
            };
            if explicit {
                lexer.take_command();
            }
            // Where the cursor stood before this command consumed anything.
            //
            // With closepath refused above, this guard is unreachable through
            // `parse` as it stands, and the invariant that makes it so is
            // worth stating because it is what a new arm would break: an
            // implicit repeat can now only be a moveto, a lineto or a curve,
            // each of which calls `point`, which either advances the cursor or
            // ends the parse. An arm added below that consumes no bytes would
            // spin without this — which is the hang closepath had, arriving by
            // a second route.
            let before = lexer.at;
            command = Some(op);
            match op {
                'M' | 'm' => {
                    let p = lexer.point(op == 'm', at)?;
                    if current.len() > 2 {
                        subpaths.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                    at = p;
                    start = p;
                    current.push(p);
                    // A second coordinate pair after a moveto is a lineto,
                    // per the SVG grammar.
                    command = Some(if op == 'm' { 'l' } else { 'L' });
                }
                'L' | 'l' => {
                    let p = lexer.point(op == 'l', at)?;
                    at = p;
                    current.push(p);
                }
                'C' | 'c' => {
                    let relative = op == 'c';
                    let c1 = lexer.point(relative, at)?;
                    let c2 = lexer.point(relative, at)?;
                    let end = lexer.point(relative, at)?;
                    flatten_cubic(at, c1, c2, end, &mut current);
                    at = end;
                }
                'Z' | 'z' => {
                    if current.len() > 2 {
                        subpaths.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                    at = start;
                }
                _ => return None,
            }
            if lexer.at == before && !explicit {
                return None;
            }
        }
        if current.len() > 2 {
            subpaths.push(current);
        }
        if subpaths.is_empty() || viewbox <= 0.0 {
            return None;
        }
        for ring in &mut subpaths {
            for p in ring.iter_mut() {
                p[0] /= viewbox;
                p[1] /= viewbox;
            }
        }
        Some(Self { subpaths })
    }

    /// How many closed subpaths the mark parsed into.
    ///
    /// Read by `the_shipped_mark_parses`, which holds it against the count the
    /// design system's own test pins, so a re-export that changes the geometry
    /// reddens here as well as there.
    #[must_use]
    pub fn subpath_count(&self) -> usize {
        self.subpaths.len()
    }

    /// The coverage mask: `MASK_SIZE` square, white, alpha carrying the
    /// even-odd fill of the outline.
    ///
    /// Public so a test can read the coverage back rather than photograph a
    /// window: the mark's alpha at the centre of a tooth and at the centre of
    /// the gap beside it decides whether the fill rule ran the right way
    /// round, and no baseline is needed to ask that.
    #[must_use]
    pub fn mask(&self) -> egui::ColorImage {
        let mut alpha = vec![0u16; MASK_SIZE * MASK_SIZE];
        let samples = MASK_SIZE * SUPERSAMPLE;
        let step = 1.0 / samples as f32;
        // Every edge of every ring, as (y0, y1, x-at-y0, dx/dy), so the row
        // loop below does no per-row branching on ring boundaries.
        let mut edges: Vec<Edge> = Vec::new();
        for ring in &self.subpaths {
            for (a, b) in ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .take(ring.len())
            {
                if (a[1] - b[1]).abs() < f32::EPSILON {
                    continue;
                }
                let (top, bottom) = if a[1] < b[1] { (a, b) } else { (b, a) };
                edges.push(Edge {
                    y0: top[1],
                    y1: bottom[1],
                    x0: top[0],
                    slope: (bottom[0] - top[0]) / (bottom[1] - top[1]),
                });
            }
        }
        let mut crossings: Vec<f32> = Vec::new();
        for sy in 0..samples {
            let y = (sy as f32 + 0.5) * step;
            crossings.clear();
            for e in &edges {
                if y >= e.y0 && y < e.y1 {
                    crossings.push(e.x0 + (y - e.y0) * e.slope);
                }
            }
            if crossings.len() < 2 {
                continue;
            }
            crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let row = sy / SUPERSAMPLE;
            for pair in crossings.chunks_exact(2) {
                let (from, to) = (pair[0], pair[1]);
                let first = ((from / step).ceil().max(0.0)) as usize;
                let last = ((to / step).floor().min(samples as f32)) as usize;
                for sx in first..last {
                    alpha[row * MASK_SIZE + sx / SUPERSAMPLE] += 1;
                }
            }
        }
        let full = (SUPERSAMPLE * SUPERSAMPLE) as u16;
        let pixels = alpha
            .iter()
            .map(|&hits| {
                let a = (u32::from(hits.min(full)) * 255 / u32::from(full)) as u8;
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, a)
            })
            .collect();
        egui::ColorImage::new([MASK_SIZE, MASK_SIZE], pixels)
    }
}

/// One edge of the flattened outline, in the form the scanline loop wants.
struct Edge {
    y0: f32,
    y1: f32,
    x0: f32,
    slope: f32,
}

/// The mark's coverage mask, rasterised once for the life of the process.
///
/// A [`egui::ColorImage`] rather than a texture, deliberately: a
/// [`egui::TextureHandle`] belongs to the [`egui::Context`] that loaded it, so
/// a handle cached in a `static` is valid in the first context to ask for one
/// and stale in every context after — which in a test binary means the first
/// test draws the mark and the rest draw nothing. The image is context-free;
/// the handle is [`MeridianApp`](crate::window::MeridianApp)'s, loaded beside
/// the door's thumbnails and dropped with the window.
///
/// `None` when the design system's path did not parse — see [`mark`].
pub fn image() -> Option<&'static egui::ColorImage> {
    static IMAGE: OnceLock<Option<egui::ColorImage>> = OnceLock::new();
    IMAGE.get_or_init(|| Some(mark()?.mask())).as_ref()
}

/// Load the mark into `ctx`, or `None` if the design system's path did not
/// parse.
pub fn load(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    Some(ctx.load_texture(
        "meridian-mark",
        image()?.clone(),
        egui::TextureOptions::LINEAR,
    ))
}

/// Paint the mark into `rect` in `ink`.
///
/// The mask is square and centred in `rect`'s shorter side, so a caller may
/// hand this the row it has and get the mark at the size that row allows
/// without computing the square itself. A caller with no texture — the design
/// system's path did not parse — draws nothing rather than a placeholder: the
/// mark is identity, and a stand-in for it is worse than its absence.
pub fn paint(
    painter: &egui::Painter,
    texture: Option<&egui::TextureHandle>,
    rect: egui::Rect,
    ink: egui::Color32,
) {
    let Some(tex) = texture else {
        return;
    };
    let side = rect.width().min(rect.height());
    let square = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
    painter.image(
        tex.id(),
        square,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        ink,
    );
}

/// Subdivide one cubic into [`CUBIC_SEGMENTS`] line segments, appending each
/// point after the first — the caller already holds the current point.
fn flatten_cubic(
    from: [f32; 2],
    c1: [f32; 2],
    c2: [f32; 2],
    to: [f32; 2],
    out: &mut Vec<[f32; 2]>,
) {
    for step in 1..=CUBIC_SEGMENTS {
        let t = step as f32 / CUBIC_SEGMENTS as f32;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push([
            a * from[0] + b * c1[0] + c * c2[0] + d * to[0],
            a * from[1] + b * c1[1] + c * c2[1] + d * to[1],
        ]);
    }
}

/// The SVG path grammar this module implements, as a cursor over the string.
struct Lexer<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Lexer<'a> {
    fn new(path: &'a str) -> Self {
        Self {
            bytes: path.as_bytes(),
            at: 0,
        }
    }

    /// Skip whitespace and commas, which the grammar treats alike.
    fn skip_separators(&mut self) {
        while self.at < self.bytes.len() {
            match self.bytes[self.at] {
                b' ' | b',' | b'\t' | b'\n' | b'\r' => self.at += 1,
                _ => break,
            }
        }
    }

    /// The next command letter, or `None` for a coordinate that repeats the
    /// previous one. The outer `Option` is end-of-input.
    fn peek_command(&mut self) -> Option<Option<char>> {
        self.skip_separators();
        let b = *self.bytes.get(self.at)?;
        if b.is_ascii_alphabetic() {
            Some(Some(b as char))
        } else {
            Some(None)
        }
    }

    fn take_command(&mut self) {
        self.at += 1;
    }

    /// One coordinate pair, made absolute against `at` when `relative`.
    fn point(&mut self, relative: bool, at: [f32; 2]) -> Option<[f32; 2]> {
        let x = self.number()?;
        let y = self.number()?;
        Some(if relative {
            [at[0] + x, at[1] + y]
        } else {
            [x, y]
        })
    }

    /// One number. The export writes exponents (`4.9e-14`) and elides the
    /// separator before a leading minus, so both are handled here rather than
    /// by splitting the string on whitespace.
    fn number(&mut self) -> Option<f32> {
        self.skip_separators();
        let start = self.at;
        if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
            self.at += 1;
        }
        while matches!(self.bytes.get(self.at), Some(b) if b.is_ascii_digit() || *b == b'.') {
            self.at += 1;
        }
        if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            while matches!(self.bytes.get(self.at), Some(b) if b.is_ascii_digit()) {
                self.at += 1;
            }
        }
        if self.at == start {
            return None;
        }
        std::str::from_utf8(&self.bytes[start..self.at])
            .ok()?
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The viewbox the fixtures below are written against — a unit square, so
    /// a parsed point reads back as the number that was typed.
    const UNIT: f32 = 1.0;

    /// A square, closed, in the two forms the export uses: absolute moveto and
    /// relative linetos. The baseline the refusals below are refusals *from*.
    #[test]
    fn a_closed_square_parses_into_one_ring() {
        let mark = Mark::parse("M0,0 l1,0 l0,1 l-1,0 Z", UNIT).expect("a square is a path");
        assert_eq!(mark.subpath_count(), 1);
    }

    /// **The hang.** A coordinate pair with no command letter repeats the
    /// previous command, and closepath consumes no coordinates — so before the
    /// refusal this re-derived the same `Z` against an unmoved cursor for ever.
    /// It is not merely unhandled input: the SVG grammar has no implicit-repeat
    /// form for closepath, so there is nothing here to be permissive about.
    #[test]
    fn a_bare_pair_after_closepath_is_refused() {
        assert!(Mark::parse("M0,0 l1,0 l0,1 Z 5,5", UNIT).is_none());
    }

    /// The same refusal for the lowercase spelling, which is a separate match
    /// arm and would be a separate hang.
    #[test]
    fn a_bare_pair_after_a_lowercase_closepath_is_refused() {
        assert!(Mark::parse("M0,0 l1,0 l0,1 z 5,5", UNIT).is_none());
    }

    /// A coordinate before any command has no command to repeat.
    #[test]
    fn a_path_that_opens_with_a_coordinate_is_refused() {
        assert!(Mark::parse("5,5 l1,0", UNIT).is_none());
    }

    /// An arc is a real change to the artwork rather than something to
    /// approximate, and the parser says so instead of drawing a straight line
    /// where a curve belongs.
    #[test]
    fn an_unimplemented_command_is_refused() {
        assert!(Mark::parse("M0,0 A1,1 0 0 1 1,1 Z", UNIT).is_none());
    }

    /// A command whose coordinates run out mid-pair.
    #[test]
    fn a_truncated_coordinate_pair_is_refused() {
        assert!(Mark::parse("M0,0 l1,", UNIT).is_none());
    }

    /// Nothing to draw is not a mark.
    #[test]
    fn an_empty_path_is_refused() {
        assert!(Mark::parse("", UNIT).is_none());
        assert!(Mark::parse("   ", UNIT).is_none());
    }

    /// The export writes exponents and elides the separator before a leading
    /// minus, so both reach `number` and neither may swallow the other.
    #[test]
    fn exponents_and_elided_separators_parse() {
        let mark = Mark::parse("M0,0l1,0l0,1l-1,0Z", UNIT).expect("elided separators");
        assert_eq!(mark.subpath_count(), 1);
        let mark = Mark::parse("M4.9e-14,0 l1,0 l0,1 Z", UNIT).expect("an exponent");
        assert_eq!(mark.subpath_count(), 1);
    }

    /// A relative moveto after closepath is measured from the subpath's start
    /// and not from the last point drawn, which is what the export's five
    /// `m` commands rely on. Read back off the second ring's first point.
    #[test]
    fn a_relative_moveto_after_closepath_starts_from_the_subpath_start() {
        // Opens at (10,10), walks away, closes, then steps (1,1) — from
        // (10,10), not from the far corner.
        let mark =
            Mark::parse("M10,10 l5,0 l0,5 Z m1,1 l2,0 l0,2 Z", UNIT).expect("two closed rings");
        assert_eq!(mark.subpath_count(), 2);
        assert_eq!(mark.subpaths[1][0], [11.0, 11.0]);
    }

    /// The viewbox divides, so a degenerate one cannot produce infinities.
    #[test]
    fn a_zero_viewbox_is_refused() {
        assert!(Mark::parse("M0,0 l1,0 l0,1 Z", 0.0).is_none());
    }
}
