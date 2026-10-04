//! d3-format's number specifier — `s`, `.2s`, `.0%`, `+.1f`, `,d` — read from a
//! spec string and applied to a number.
//!
//! A Mosaic spec's `xTickFormat` / `yTickFormat` is a d3-format specifier, and
//! d3-format is the authority for what one prints. This module ports its
//! grammar (`[[fill]align][sign][symbol][0][width][,][.precision][~][type]`) and
//! its formatter, and d3-scale's `tickFormat` rule for a specifier that names no
//! precision: the precision is inferred from the step between ticks, so a `%`
//! axis stepping by 0.2 prints `20%` and one stepping by 0.02 prints `2%`.
//!
//! One judge serves the parser and the renderer. [`NumberFormat::parse`] decides
//! what a valid format is, the parse warning for a bad one asks it, and the axis
//! draws with what it returns, so the warning and the drawing cannot disagree.
//!
//! The English locale is the only one: `,` groups thousands, `.` is the decimal
//! point, `$` is a prefix, and a negative number leads with U+2212 (the minus
//! sign d3-format prints), not a hyphen.
//!
//! Rounding matches JavaScript, which d3-format runs on: a value exactly half
//! way between two results rounds up (`2.5` is `3`, and `.1s` of `2500` is
//! `3k`), where Rust's own formatter rounds half to even. The digits are taken
//! from the exact decimal expansion of the double, so no tie is missed or
//! invented.

/// One SI prefix per three decades, from 10⁻²⁴ (`y`) to 10²⁴ (`Y`).
const SI_PREFIXES: [&str; 17] = [
    "y", "z", "a", "f", "p", "n", "µ", "m", "", "k", "M", "G", "T", "P", "E", "Z", "Y",
];

/// The sign d3-format prints in front of a negative number.
const MINUS: &str = "\u{2212}";

/// The types d3-format formats by name. Any other letter is an alias for
/// `.12~g`, and `n` is `,g`.
const NAMED_TYPES: &str = "%bcdefgoprsXx";

/// How the padding of a `width` sits against the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    /// `<` — the number first, padding after.
    Left,
    /// `>` — padding first. The default.
    Right,
    /// `^` — padding on both sides.
    Centre,
    /// `=` — padding between the sign and the digits.
    AfterSign,
}

/// The `sign` flag: how a number's sign is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sign {
    /// `-` — only a negative number has one. The default.
    Minus,
    /// `+` — a positive number gets `+`.
    Plus,
    /// `(` — a negative number is wrapped in parentheses.
    Parens,
    /// ` ` — a positive number gets a space.
    Space,
}

/// The `symbol` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Symbol {
    None,
    /// `$` — a currency prefix.
    Currency,
    /// `#` — a `0b` / `0o` / `0x` prefix on the integer bases.
    Alternate,
}

/// A parsed d3-format number specifier.
///
/// The precision is kept as the analyst wrote it, or absent, so
/// [`NumberFormat::tick_format`] can tell a specifier that names a precision
/// from one that leaves it to the axis. d3-format's aliases (`n`, an empty type,
/// an unknown letter) are applied when a number is formatted, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberFormat {
    fill: char,
    align: Align,
    sign: Sign,
    symbol: Symbol,
    zero: bool,
    width: Option<usize>,
    comma: bool,
    /// Signed because the axis's inference can land below zero (`%` on a 0.2
    /// step reads `-1`); it is clamped where a number is formatted.
    precision: Option<i32>,
    trim: bool,
    /// `None` is the empty type, which d3-format formats as `.12~g`.
    ty: Option<char>,
}

impl NumberFormat {
    /// Read a d3-format specifier, or `None` when it is not one.
    ///
    /// The grammar is d3-format's, character for character:
    /// `[[fill]align][sign][symbol][0][width][,][.precision][~][type]`, every
    /// part optional and in that order, so `""`, `"s"`, `".2s"`, `"+.1f"`,
    /// `"$,.2f"`, `"_>8d"` and `"q"` are all valid and `"~~"` and `".f"` are
    /// not. A date format such as `%b` is not one either.
    #[must_use]
    pub fn parse(spec: &str) -> Option<Self> {
        let chars: Vec<char> = spec.chars().collect();
        let mut at = 0;
        let next = |at: usize| chars.get(at).copied();
        let is_align = |c: char| "<>=^".contains(c);
        let is_line_end = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');

        let mut fill = ' ';
        let mut align = Align::Right;
        let to_align = |c: char| match c {
            '<' => Align::Left,
            '>' => Align::Right,
            '^' => Align::Centre,
            _ => Align::AfterSign,
        };
        // `(.)?([<>=^])`: a fill character only when an align character
        // follows it. `.` in d3-format's pattern is any character but a line
        // break.
        match (next(0), next(1)) {
            (Some(f), Some(a)) if is_align(a) && !is_line_end(f) => {
                fill = f;
                align = to_align(a);
                at = 2;
            }
            (Some(a), _) if is_align(a) => {
                align = to_align(a);
                at = 1;
            }
            _ => {}
        }

        let sign = match next(at) {
            Some('+') => Some(Sign::Plus),
            Some('-') => Some(Sign::Minus),
            Some('(') => Some(Sign::Parens),
            Some(' ') => Some(Sign::Space),
            _ => None,
        };
        if sign.is_some() {
            at += 1;
        }

        let symbol = match next(at) {
            Some('$') => Symbol::Currency,
            Some('#') => Symbol::Alternate,
            _ => Symbol::None,
        };
        if symbol != Symbol::None {
            at += 1;
        }

        let zero = next(at) == Some('0');
        if zero {
            at += 1;
        }

        let digits = |at: &mut usize| -> Option<usize> {
            let start = *at;
            while next(*at).is_some_and(|c| c.is_ascii_digit()) {
                *at += 1;
            }
            (*at > start).then(|| {
                // A width or precision with more digits than a `usize` holds is
                // far past every clamp below, and a width past ten thousand
                // pads a tick label to nothing a chart can show; saturate
                // rather than fail.
                chars[start..*at]
                    .iter()
                    .fold(0usize, |n, c| {
                        n.saturating_mul(10)
                            .saturating_add(c.to_digit(10).unwrap_or(0) as usize)
                    })
                    .min(10_000)
            })
        };
        let width = digits(&mut at);

        let comma = next(at) == Some(',');
        if comma {
            at += 1;
        }

        let precision = if next(at) == Some('.') {
            at += 1;
            // `.` must be followed by digits, or the specifier is invalid.
            Some(i32::try_from(digits(&mut at)?).unwrap_or(i32::MAX))
        } else {
            None
        };

        let trim = next(at) == Some('~');
        if trim {
            at += 1;
        }

        let ty = match next(at) {
            Some(c) if c.is_ascii_alphabetic() || c == '%' => {
                at += 1;
                Some(c)
            }
            _ => None,
        };

        (at == chars.len()).then_some(Self {
            fill,
            align,
            sign: sign.unwrap_or(Sign::Minus),
            symbol,
            zero,
            width,
            comma,
            precision,
            trim,
            ty,
        })
    }

    /// Whether this specifier names a precision (`.2s`) rather than leaving it
    /// to the axis (`s`).
    #[must_use]
    pub fn has_precision(&self) -> bool {
        self.precision.is_some()
    }

    /// Format `value` as d3-format does.
    #[must_use]
    pub fn format(&self, value: f64) -> String {
        self.format_with_suffix(value, "")
    }

    /// The format for the ticks of a linear axis running from `start` to `stop`
    /// with `step` between ticks, as d3-scale's `tickFormat` builds it.
    ///
    /// A specifier that names a precision is the analyst's own and prints each
    /// value as d3-format does, except under `s`. One that does not gets its
    /// precision from the step, so no two ticks read alike and none carries
    /// digits the step does not need:
    ///
    /// * `s` takes one SI prefix for the whole axis, from the larger end of the
    ///   domain, and the decimals the step needs: ticks 0 to 2000 by 500 read
    ///   `0.0k`, `0.5k`, `1.0k`, `1.5k` and `2.0k`. A precision the specifier
    ///   names replaces the decimals and leaves the shared prefix, as
    ///   d3-scale's `tickFormat` does: `.2s` over 0 to 2000 by 500 reads
    ///   `0.00k`, `0.50k`, `1.00k`, `1.50k` and `2.00k`;
    /// * `f` and `%` take the decimals the step needs: 0 to 1 by 0.2 under `%`
    ///   reads `0%`, `20%` … `100%`;
    /// * the empty type, `e`, `g`, `p` and `r` take the significant digits the
    ///   larger end and the step need;
    /// * any other type has no precision to infer.
    #[must_use]
    pub fn tick_format(self, start: f64, stop: f64, step: f64) -> TickFormat {
        if self.precision.is_some() && self.ty != Some('s') {
            return TickFormat::plain(self);
        }
        let biggest = start.abs().max(stop.abs());
        let mut format = self;
        match self.ty {
            Some('s') => {
                if self.precision.is_none() {
                    format.precision = precision_prefix(step, biggest);
                }
                // A domain whose larger end is zero has no prefix to choose,
                // and no ticks to draw a prefix on.
                let Some(exponent) = exponent10(biggest) else {
                    return TickFormat::plain(self);
                };
                format.ty = Some('f');
                return TickFormat {
                    format,
                    prefix_exponent: Some(exponent.div_euclid(3).clamp(-8, 8) * 3),
                };
            }
            None | Some('e' | 'g' | 'p' | 'r') => {
                format.precision =
                    precision_round(step, biggest).map(|p| p - i32::from(self.ty == Some('e')));
            }
            Some(ty @ ('f' | '%')) => {
                format.precision = precision_fixed(step).map(|p| p - if ty == '%' { 2 } else { 0 });
            }
            Some(_) => {}
        }
        TickFormat::plain(format)
    }

    /// The format for the ticks of a log or symlog axis, which sit on decades
    /// and not on a step.
    ///
    /// d3-scale's log scale trims insignificant zeros when the specifier names
    /// no precision, so the decades read `1`, `10`, `100` and `1k`, not
    /// `1.00000`.
    #[must_use]
    pub fn decade_format(mut self) -> TickFormat {
        if self.precision.is_none() {
            self.trim = true;
        }
        TickFormat::plain(self)
    }

    /// `format`, with `extra_suffix` written after the number and inside the
    /// padding — where d3-format's `formatPrefix` puts its SI prefix.
    fn format_with_suffix(&self, value: f64, extra_suffix: &str) -> String {
        let plan = self.plan();
        let ty = plan.ty;

        let mut prefix = match self.symbol {
            Symbol::Currency => "$".to_string(),
            Symbol::Alternate if "boxX".contains(ty) => format!("0{}", ty.to_ascii_lowercase()),
            _ => String::new(),
        };
        let mut suffix = match self.symbol {
            Symbol::Currency => String::new(),
            _ if "%p".contains(ty) => "%".to_string(),
            _ => String::new(),
        };
        suffix.push_str(extra_suffix);

        let mut body;
        if ty == 'c' {
            suffix = format!("{value}{suffix}");
            body = String::new();
        } else {
            let mut negative = value < 0.0 || (value == 0.0 && value.is_sign_negative());
            let mut si_prefix = None;
            body = if value.is_nan() {
                "NaN".to_string()
            } else {
                let (text, exponent) = format_type(ty, value.abs(), plan.precision);
                si_prefix = exponent;
                text
            };
            if plan.trim {
                body = trim_zeros(&body);
            }
            // A negative that rounds to zero prints as zero, unless the
            // analyst asked for the sign of every number.
            if negative && body.parse::<f64>().is_ok_and(|n| n == 0.0) && self.sign != Sign::Plus {
                negative = false;
            }
            let sign_text = if negative {
                if self.sign == Sign::Parens {
                    "("
                } else {
                    MINUS
                }
            } else {
                match self.sign {
                    Sign::Plus => "+",
                    Sign::Space => " ",
                    Sign::Minus | Sign::Parens => "",
                }
            };
            prefix.insert_str(0, sign_text);
            let unit = match si_prefix {
                Some(exponent) if ty == 's' && body != "NaN" => {
                    SI_PREFIXES[usize::try_from(8 + exponent / 3).unwrap_or(8)]
                }
                _ => "",
            };
            suffix = format!(
                "{unit}{suffix}{}",
                if negative && self.sign == Sign::Parens {
                    ")"
                } else {
                    ""
                }
            );
            // The integer part is what grouping and padding act on; a
            // fraction or an exponent is not grouped.
            if "defgprs%".contains(ty) {
                if let Some(at) = body.find(|c: char| !c.is_ascii_digit()) {
                    let rest = &body[at..];
                    let tail = rest.strip_prefix('.').unwrap_or(rest);
                    let point = if rest.starts_with('.') { "." } else { "" };
                    suffix = format!("{point}{tail}{suffix}");
                    body.truncate(at);
                }
            }
        }

        // Grouping runs before padding, unless the fill is zero, when the
        // padding is part of what is grouped.
        if plan.comma && !plan.zero {
            body = group(&body, usize::MAX);
        }
        let width = self.width.unwrap_or(0);
        let length = prefix.chars().count() + body.chars().count() + suffix.chars().count();
        let mut padding: Vec<char> = vec![plan.fill; width.saturating_sub(length)];
        if plan.comma && plan.zero {
            let padded: String = padding.iter().collect::<String>() + &body;
            let limit = if padding.is_empty() {
                usize::MAX
            } else {
                width.saturating_sub(suffix.chars().count())
            };
            body = group(&padded, limit);
            padding.clear();
        }
        let pad: String = padding.iter().collect();
        match plan.align {
            Align::Left => format!("{prefix}{body}{suffix}{pad}"),
            Align::AfterSign => format!("{prefix}{pad}{body}{suffix}"),
            Align::Centre => {
                let half = padding.len() / 2;
                let before: String = padding[..half].iter().collect();
                let after: String = padding[half..].iter().collect();
                format!("{before}{prefix}{body}{suffix}{after}")
            }
            Align::Right => format!("{pad}{prefix}{body}{suffix}"),
        }
    }

    /// The specifier with d3-format's aliases and defaults applied.
    fn plan(&self) -> Plan {
        let (mut ty, mut comma, mut trim, mut precision) =
            (self.ty, self.comma, self.trim, self.precision);
        match ty {
            // `n` is `,g`.
            Some('n') => {
                comma = true;
                ty = Some('g');
            }
            Some(c) if NAMED_TYPES.contains(c) => {}
            // The empty type, and any letter that is no type, is `.12~g`.
            _ => {
                precision.get_or_insert(12);
                trim = true;
                ty = Some('g');
            }
        }
        let ty = ty.unwrap_or('g');
        // Zero fill puts the padding after the sign and before the digits.
        let zero = self.zero || (self.fill == '0' && self.align == Align::AfterSign);
        let precision = match precision {
            None => 6,
            Some(p) if "gprs".contains(ty) => p.clamp(1, 21),
            Some(p) => p.clamp(0, 20),
        };
        Plan {
            ty,
            comma,
            trim,
            zero,
            fill: if zero { '0' } else { self.fill },
            align: if zero { Align::AfterSign } else { self.align },
            precision: usize::try_from(precision).unwrap_or(0),
        }
    }
}

/// A number format with the axis's inference applied — what an axis draws its
/// tick text with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickFormat {
    format: NumberFormat,
    /// The power of ten, a multiple of three, an SI-prefixed axis divides every
    /// tick by. `None` for an axis with no shared prefix.
    prefix_exponent: Option<i32>,
}

impl TickFormat {
    fn plain(format: NumberFormat) -> Self {
        Self {
            format,
            prefix_exponent: None,
        }
    }

    /// The text of the tick at `value`.
    #[must_use]
    pub fn format(&self, value: f64) -> String {
        match self.prefix_exponent {
            None => self.format.format(value),
            Some(exponent) => {
                let unit = SI_PREFIXES[usize::try_from(8 + exponent / 3).unwrap_or(8)];
                self.format
                    .format_with_suffix(10f64.powi(-exponent) * value, unit)
            }
        }
    }
}

/// A specifier with its aliases resolved and its precision clamped.
struct Plan {
    ty: char,
    comma: bool,
    trim: bool,
    zero: bool,
    fill: char,
    align: Align,
    precision: usize,
}

// ---------------------------------------------------------------------------
// d3-format's `precisionFixed`, `precisionPrefix` and `precisionRound`
// ---------------------------------------------------------------------------

/// The decimal exponent of `x`'s first significant digit, as d3-format's
/// `exponent` reads it from the shortest round-trip digits. `None` for zero and
/// for a number that is not finite.
fn exponent10(x: f64) -> Option<i32> {
    let x = x.abs();
    if x == 0.0 || !x.is_finite() {
        return None;
    }
    format!("{x:e}").split_once('e')?.1.parse().ok()
}

/// The decimals a fixed-point tick of this `step` needs.
fn precision_fixed(step: f64) -> Option<i32> {
    Some((-exponent10(step)?).max(0))
}

/// The decimals an SI-prefixed tick of this `step` needs, when the prefix is
/// chosen from `value`.
fn precision_prefix(step: f64, value: f64) -> Option<i32> {
    let prefix = exponent10(value)?.div_euclid(3).clamp(-8, 8) * 3;
    Some((prefix - exponent10(step)?).max(0))
}

/// The significant digits a tick of this `step` needs when the largest tick is
/// `max`.
fn precision_round(step: f64, max: f64) -> Option<i32> {
    let step = step.abs();
    let max = max.abs() - step;
    Some((exponent10(max)? - exponent10(step)?).max(0) + 1)
}

// ---------------------------------------------------------------------------
// d3-format's types, over JavaScript's rounding
// ---------------------------------------------------------------------------

/// `x` (non-negative) formatted as `ty` with `precision`, and the SI exponent
/// when `ty` is `s`. What d3-format's `formatTypes` return.
fn format_type(ty: char, x: f64, precision: usize) -> (String, Option<i32>) {
    if !x.is_finite() {
        return (if ty == 'd' { "∞" } else { "Infinity" }.to_string(), None);
    }
    let text = match ty {
        '%' => to_fixed(x * 100.0, precision),
        'd' if x.round() >= JS_EXPONENT_FLOOR => large_plain_text(x.round()),
        'd' => to_fixed(x.round(), 0),
        'e' => to_exponential(x, precision),
        'f' => to_fixed(x, precision),
        'g' => to_precision(x, precision),
        'p' => format_rounded(x * 100.0, precision),
        'r' => format_rounded(x, precision),
        's' => return format_prefix_auto(x, precision),
        'b' => format!("{:b}", x.round() as u128),
        'o' => format!("{:o}", x.round() as u128),
        'x' => format!("{:x}", x.round() as u128),
        'X' => format!("{:X}", x.round() as u128),
        _ => format!("{x}"),
    };
    (text, None)
}

/// The exact decimal digits of `x` (positive, finite) and the exponent of the
/// first: `x = d0.d1d2… × 10^exponent`.
///
/// A double holds at most 767 significant decimal digits, so asking for 780
/// gives the value exactly and pads with zeros.
fn exact_digits(x: f64) -> (Vec<u8>, i32) {
    let text = format!("{x:.780e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    (digits, exponent.parse().unwrap_or(0))
}

/// Add one to the last of `digits`, carrying leftwards; a carry out of the
/// front grows the number by a digit.
fn increment(digits: &mut Vec<u8>) {
    for digit in digits.iter_mut().rev() {
        if *digit == 9 {
            *digit = 0;
        } else {
            *digit += 1;
            return;
        }
    }
    digits.insert(0, 1);
}

/// `x` rounded to `n` significant digits, half up, as the `n` digits and the
/// exponent of the first. Zero is `n` zeros at exponent 0.
fn round_significant(x: f64, n: usize) -> (Vec<u8>, i32) {
    if x == 0.0 {
        return (vec![0; n], 0);
    }
    let (mut digits, mut exponent) = exact_digits(x);
    let round_up = digits.get(n).is_some_and(|d| *d >= 5);
    digits.resize(n, 0);
    if round_up {
        increment(&mut digits);
        if digits.len() > n {
            // 9.99… became 10.0…: one digit more, one place higher.
            digits.truncate(n);
            exponent += 1;
        }
    }
    (digits, exponent)
}

fn digit_text(digits: &[u8]) -> String {
    digits.iter().map(|d| char::from(b'0' + d)).collect()
}

/// The point at and above which JavaScript stops writing a number in full:
/// `toFixed` returns its `toString`, an exponential, and `toString` itself
/// does the same.
const JS_EXPONENT_FLOOR: f64 = 1e21;

/// `x` (at least [`JS_EXPONENT_FLOOR`]) as JavaScript's `toString` writes it:
/// the shortest round-trip digits and an exponent, `1e+21`, `1.2345e+25`.
fn large_exponent_text(x: f64) -> String {
    let text = format!("{x:e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    format!("{mantissa}e+{exponent}")
}

/// `x` (a whole number at least [`JS_EXPONENT_FLOOR`]) written out in full from
/// its shortest round-trip digits, padded with zeros, as `toLocaleString`
/// writes it for d3-format's `d`. The double's exact expansion is not what it
/// prints: `1e23` is `100000000000000000000000`, not the binary neighbour
/// below it.
fn large_plain_text(x: f64) -> String {
    let text = format!("{x:e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: usize = exponent.parse().unwrap_or(0);
    let zeros = (exponent + 1).saturating_sub(digits.len());
    format!("{digits}{}", "0".repeat(zeros))
}

/// JavaScript's `x.toFixed(places)` for a non-negative finite `x`.
fn to_fixed(x: f64, places: usize) -> String {
    if x >= JS_EXPONENT_FLOOR {
        return large_exponent_text(x);
    }
    let mut whole: Vec<u8> = Vec::new();
    if x != 0.0 {
        let (digits, exponent) = exact_digits(x);
        // The digits at or above 10^-places, once x is scaled by 10^places.
        if let Ok(keep) = usize::try_from(exponent + i32::try_from(places).unwrap_or(0) + 1) {
            whole = digits.iter().copied().take(keep).collect();
            whole.resize(keep, 0);
            if digits.get(keep).is_some_and(|d| *d >= 5) {
                increment(&mut whole);
            }
        }
    }
    if whole.is_empty() {
        whole.push(0);
    }
    let text = digit_text(&whole);
    if places == 0 {
        return text;
    }
    let padded = format!("{text:0>width$}", width = places + 1);
    let (int, frac) = padded.split_at(padded.len() - places);
    format!("{int}.{frac}")
}

/// `digits` `e` `exponent`, as JavaScript writes an exponential number.
fn exponent_text(digits: &[u8], exponent: i32) -> String {
    let (first, rest) = digits.split_at(1);
    let point = if rest.is_empty() { "" } else { "." };
    let sign = if exponent < 0 { '-' } else { '+' };
    format!(
        "{}{point}{}e{sign}{}",
        digit_text(first),
        digit_text(rest),
        exponent.abs()
    )
}

/// JavaScript's `x.toExponential(places)`.
fn to_exponential(x: f64, places: usize) -> String {
    let (digits, exponent) = round_significant(x, places + 1);
    exponent_text(&digits, exponent)
}

/// JavaScript's `x.toPrecision(significant)`, `significant` at least one.
fn to_precision(x: f64, significant: usize) -> String {
    let (digits, exponent) = round_significant(x, significant);
    let n = i32::try_from(significant).unwrap_or(i32::MAX);
    if !(-6..n).contains(&exponent) {
        return exponent_text(&digits, exponent);
    }
    let text = digit_text(&digits);
    if exponent == n - 1 {
        text
    } else if exponent >= 0 {
        let at = usize::try_from(exponent).unwrap_or(0) + 1;
        format!("{}.{}", &text[..at], &text[at..])
    } else {
        format!(
            "0.{}{text}",
            "0".repeat(usize::try_from(-(exponent + 1)).unwrap_or(0))
        )
    }
}

/// d3-format's `formatRounded`: `x` to `significant` digits, in fixed notation.
fn format_rounded(x: f64, significant: usize) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let (digits, exponent) = round_significant(x, significant);
    let text = digit_text(&digits);
    if exponent < 0 {
        return format!(
            "0.{}{text}",
            "0".repeat(usize::try_from(-exponent - 1).unwrap_or(0))
        );
    }
    let at = usize::try_from(exponent).unwrap_or(0) + 1;
    if text.len() > at {
        format!("{}.{}", &text[..at], &text[at..])
    } else {
        format!("{text}{}", "0".repeat(at - text.len()))
    }
}

/// d3-format's `formatPrefixAuto`, the `s` type: `x` to `significant` digits
/// scaled to the nearest SI prefix, and that prefix's exponent.
fn format_prefix_auto(x: f64, significant: usize) -> (String, Option<i32>) {
    if x == 0.0 {
        return (to_precision(x, significant), None);
    }
    let (digits, exponent) = round_significant(x, significant);
    let prefix = exponent.div_euclid(3).clamp(-8, 8) * 3;
    let coefficient = digit_text(&digits);
    let point = exponent - prefix + 1;
    let n = i32::try_from(coefficient.len()).unwrap_or(i32::MAX);
    let text = if point == n {
        coefficient
    } else if point > n {
        format!(
            "{coefficient}{}",
            "0".repeat(usize::try_from(point - n).unwrap_or(0))
        )
    } else if point > 0 {
        let at = usize::try_from(point).unwrap_or(0);
        format!("{}.{}", &coefficient[..at], &coefficient[at..])
    } else {
        // Smaller than the smallest prefix, 1y.
        let keep = usize::try_from((i32::try_from(significant).unwrap_or(0) + point - 1).max(0))
            .unwrap_or(0);
        let tail = if keep == 0 {
            shortest_digits(x)
        } else {
            digit_text(&round_significant(x, keep).0)
        };
        format!(
            "0.{}{tail}",
            "0".repeat(usize::try_from(-point).unwrap_or(0))
        )
    };
    (text, Some(prefix))
}

/// The shortest round-trip digits of `x` (positive, finite).
fn shortest_digits(x: f64) -> String {
    let text = format!("{x:e}");
    let mantissa = text.split_once('e').map_or(text.as_str(), |(m, _)| m);
    mantissa.chars().filter(char::is_ascii_digit).collect()
}

/// d3-format's `formatTrim`: drop the insignificant zeros of a decimal or
/// exponential number, and the point they leave bare.
fn trim_zeros(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let (mut first_zero, mut last_zero): (isize, isize) = (-1, 0);
    let mut i = 1;
    while i < chars.len() {
        let at = i as isize;
        match chars[i] {
            '.' => {
                first_zero = at;
                last_zero = at;
            }
            '0' => {
                if first_zero == 0 {
                    first_zero = at;
                }
                last_zero = at;
            }
            c if c.is_ascii_digit() => {
                if first_zero > 0 {
                    first_zero = 0;
                }
            }
            _ => break,
        }
        i += 1;
    }
    if first_zero > 0 {
        let (from, to) = (first_zero as usize, last_zero as usize + 1);
        chars[..from].iter().chain(&chars[to..]).collect()
    } else {
        text.to_string()
    }
}

/// d3-format's grouping: `,` between each three digits from the right, stopping
/// once `width` characters are used (`usize::MAX` for no limit).
fn group(value: &str, width: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut end = chars.len();
    let mut parts: Vec<String> = Vec::new();
    let mut size = 3usize;
    let mut length = 0usize;
    while end > 0 && size > 0 {
        if length.saturating_add(size).saturating_add(1) > width {
            size = width.saturating_sub(length).max(1);
        }
        let start = end.saturating_sub(size);
        parts.push(chars[start..end].iter().collect());
        end = start;
        length = length.saturating_add(size + 1);
        if length > width {
            break;
        }
        size = 3;
    }
    parts.reverse();
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(spec: &str, value: f64) -> String {
        NumberFormat::parse(spec)
            .unwrap_or_else(|| panic!("`{spec}` should parse"))
            .format(value)
    }

    /// AC1 of the card that read the tick format: a specifier that names a
    /// precision prints as d3-format prints it.
    #[test]
    fn a_format_with_a_precision_prints_as_d3_format_does() {
        assert_eq!(fmt(".2s", 1500.0), "1.5k");
        assert_eq!(fmt(".2s", 2_000_000.0), "2.0M");
        assert_eq!(fmt(".0%", 0.25), "25%");
        assert_eq!(fmt("+.1f", 3.0), "+3.0");
        assert_eq!(fmt(",d", 1500.0), "1,500");
    }

    /// The tie the exact-digit rounding exists for. `.1s` of 2500 is a tie
    /// between 2k and 3k, and JavaScript rounds it up; Rust's `{:.0e}` rounds
    /// it to the even 2.
    #[test]
    fn a_value_half_way_between_two_results_rounds_up_as_javascript_does() {
        assert_eq!(fmt(".1s", 2500.0), "3k");
        assert_eq!(fmt(".0f", 2.5), "3");
        assert_eq!(fmt(".0f", 0.5), "1");
        assert_eq!(fmt("d", 2.5), "3");
    }

    /// AC2: with no precision the axis chooses one from the step, and an SI
    /// axis shares the prefix its larger end takes.
    #[test]
    fn a_format_with_no_precision_takes_the_ticks_step() {
        let s = NumberFormat::parse("s")
            .unwrap()
            .tick_format(0.0, 2000.0, 500.0);
        let text: Vec<String> = [0.0, 500.0, 1000.0, 1500.0, 2000.0]
            .iter()
            .map(|v| s.format(*v))
            .collect();
        assert_eq!(text, ["0.0k", "0.5k", "1.0k", "1.5k", "2.0k"]);

        let pct = NumberFormat::parse("%").unwrap().tick_format(0.0, 1.0, 0.2);
        let text: Vec<String> = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0]
            .iter()
            .map(|v| pct.format(*v))
            .collect();
        assert_eq!(text, ["0%", "20%", "40%", "60%", "80%", "100%"]);
    }

    #[test]
    fn a_negative_number_leads_with_the_minus_sign_d3_format_prints() {
        assert_eq!(fmt("+f", -3.0), "\u{2212}3.000000");
        assert_eq!(fmt("(.1f", -2.5), "(2.5)");
        // A negative that rounds to zero loses its sign.
        assert_eq!(fmt(".0f", -0.4), "0");
    }

    /// The grammar of d3-format, both ends: what is a specifier and what is not.
    #[test]
    fn the_grammar_reads_what_d3_format_reads() {
        for valid in [
            "", "s", ".2s", "+f", "%", "d", ",d", "$,.2f", "_>8d", "^10d", "0=+8.1f", "q", "~s",
            "n", "+", "-", "X", "#x",
        ] {
            assert!(NumberFormat::parse(valid).is_some(), "`{valid}` is valid");
        }
        for invalid in [
            "~~", ".f", "%b", "%Y-%m", "ss", "d3", ".2.3f", "1,0d", "++f", "a b",
        ] {
            assert!(NumberFormat::parse(invalid).is_none(), "`{invalid}` is not");
        }
    }

    #[test]
    fn a_specifier_that_names_a_precision_is_told_from_one_that_does_not() {
        assert!(NumberFormat::parse(".2s").unwrap().has_precision());
        assert!(!NumberFormat::parse("s").unwrap().has_precision());
    }

    /// Every number, however large the width or precision, formats without a
    /// panic: the clamps hold at the extremes.
    #[test]
    fn extreme_widths_and_precisions_format_without_panicking() {
        for spec in [
            ".99999999999999999999f",
            "0999999999d",
            ".999s",
            ",.99e",
            "^99999d",
        ] {
            if let Some(format) = NumberFormat::parse(spec) {
                let _ = format.format(1.0e300);
                let _ = format.format(-5e-324);
                let _ = format.format(f64::NAN);
                let _ = format.format(f64::INFINITY);
            }
        }
    }
}
