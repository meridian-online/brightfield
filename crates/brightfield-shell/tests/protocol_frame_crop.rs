//! `brightfield-shot --crop` end to end, over the compiled binary itself —
//! the same shape `cli_version_help.rs` uses for `--version`/`--help` —
//! because what the card this closes promises is a **documented command**, not
//! a library function nothing outside this crate can reach the same way.
//!
//! # No digest pinned here, deliberately
//!
//! The homepage picture this flag was built for is a crop of the
//! `edgar_gleif` Protocol view, and Hugh has said that view is interim — a
//! chart is its intended successor. A test that reddens on any pixel of it
//! moving would fight a swap that is already planned, so this covers the
//! command's *properties* instead: it writes the rectangle asked for, a light
//! and a dark theme produce different bytes, two runs on one commit produce
//! the same bytes, and a rectangle that does not fit the capture is a hard
//! error rather than a silently wrong picture.
//!
//! # Why this needs a GPU, and has no skip switch
//!
//! Same reason as every other capture-path test in this crate (see
//! `dashboard_baseline.rs`'s module doc): an env-var opt-out would render "no
//! GPU here" as a passing test.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin_path() -> PathBuf {
    PathBuf::from(
        std::env::var("CARGO_BIN_EXE_brightfield-shot").expect("cargo exposes the bin path"),
    )
}

/// The workspace root, so `--spec examples/...` resolves the same way the
/// README's documented invocation does from a clean clone.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(name)
}

/// Run `brightfield-shot --spec examples/protocol/edgar_gleif/arcform.yaml
/// --scale 1.0 --theme <theme> --crop <crop> --out <out>` — the README's
/// documented command, parameterised over the two arguments these tests vary.
fn run(theme: &str, crop: &str, out: &Path) -> std::process::ExitStatus {
    Command::new(bin_path())
        .current_dir(repo_root())
        .env("BRIGHTFIELD_PROTOCOL_OFFLINE", "1")
        .args([
            "--spec",
            "examples/protocol/edgar_gleif/arcform.yaml",
            "--scale",
            "1.0",
            "--theme",
            theme,
            "--crop",
            crop,
            "--out",
        ])
        .arg(out)
        .status()
        .expect("brightfield-shot runs")
}

#[test]
fn crop_writes_a_png_of_exactly_the_named_rectangle() {
    let out = scratch("protocol_frame_crop_size.png");
    let status = run("light", "1285x815+0+0", &out);
    assert!(status.success(), "brightfield-shot --crop exited {status}");
    let img = image::open(&out).unwrap_or_else(|e| panic!("open {}: {e}", out.display()));
    assert_eq!(
        (img.width(), img.height()),
        (1285, 815),
        "the crop's own dimensions, not the full window's, must be what --out holds"
    );
}

/// A regression guard on the PNG encoding `write_png_smallest` chose, not a
/// pixel-content gate: this page ships both themes in the DOM (one hidden by
/// CSS), so a visitor's page load pays for both files, and the naive
/// `RgbaImage::save` default this crate used before encoded the same pixels
/// into roughly twice the bytes (measured: 99,576 vs 81,191 for the light
/// theme). 100 KiB is comfortably above the measured 81,191-byte output and
/// comfortably below the ~160 KiB the previous default produced, so this
/// reddens if a future change reverts the encoder choice without reddening
/// on ordinary UI drift the way a pinned byte count would.
#[test]
fn crop_output_stays_small() {
    let out = scratch("protocol_frame_crop_filesize.png");
    assert!(run("light", "1285x815+0+0", &out).success());
    let size = std::fs::metadata(&out).unwrap().len();
    assert!(
        size < 100 * 1024,
        "brightfield-protocol-light.png-shaped output grew to {size} bytes (>100 KiB) —          check write_png_smallest is still choosing CompressionType::Best + FilterType::NoFilter"
    );
}

#[test]
fn light_and_dark_crops_are_not_byte_identical() {
    let light = scratch("protocol_frame_crop_light.png");
    let dark = scratch("protocol_frame_crop_dark.png");
    assert!(run("light", "1285x815+0+0", &light).success());
    assert!(run("dark", "1285x815+0+0", &dark).success());
    let a = std::fs::read(&light).unwrap();
    let b = std::fs::read(&dark).unwrap();
    assert_ne!(
        a, b,
        "a light and a dark capture of the same spec must not be identical"
    );
}

#[test]
fn running_the_command_twice_on_one_commit_is_byte_identical() {
    let first = scratch("protocol_frame_crop_repeat_1.png");
    let second = scratch("protocol_frame_crop_repeat_2.png");
    assert!(run("light", "1285x815+0+0", &first).success());
    assert!(run("light", "1285x815+0+0", &second).success());
    let a = std::fs::read(&first).unwrap();
    let b = std::fs::read(&second).unwrap();
    assert_eq!(
        a, b,
        "two runs of the same command on the same commit must write the same bytes — \
         nothing in the capture or crop path may depend on the wall clock or on randomness"
    );
}

#[test]
fn a_crop_rectangle_that_does_not_fit_the_capture_is_a_hard_error() {
    let out = scratch("protocol_frame_crop_oob.png");
    // The protocol window over this fixture is nowhere near 9000 logical
    // points on either axis, so this rectangle cannot fit at any scale this
    // spec would plausibly render at.
    let status = run("light", "9000x9000+0+0", &out);
    assert!(
        !status.success(),
        "an out-of-bounds --crop must exit non-zero rather than write a wrong picture"
    );
    assert!(
        !out.exists(),
        "a refused crop must not leave a file behind for a caller to mistake for success"
    );
}
