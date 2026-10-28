//! **A chart handed over as a picture**: the plot under the pointer, drawn as
//! the headless export draws it, copied to the clipboard or saved as a PNG.
//!
//! The picture is [`render_export`]'s, the one `brightfield export png` writes:
//! the Vello scene with each legend drawn where the window draws it. Hover
//! chrome — the crosshair, the readout, the focus ring, the shelf band — is the
//! window's egui layer over the raster and is not in the scene, so it is absent
//! from the picture by construction rather than by being hidden first.
//!
//! A page with several plots is drawn whole and cut to the one plot asked for,
//! with the band its legend stands in beside or under it ([`plot_extent`]). The
//! cut is of the drawn picture rather than a second composition at another
//! size, so what is handed over is what the tile shows, at [`PICTURE_SCALE`].

use std::path::{Path, PathBuf};

use brightfield_spec::layout::Rect;

use crate::capture::{render_export, DEFAULT_SCALE};
use crate::pipeline::Composed;

/// Device pixels per logical point in a picture the window hands over: the
/// headless export's own default, so a picture copied from the window and one
/// written by `brightfield export png` are the same size.
pub const PICTURE_SCALE: f32 = DEFAULT_SCALE;

/// The region of the page plot `plot` takes, on the page plane: its own box,
/// joined with the band its legend is drawn in at its right or under it. `None`
/// for an index the composition has no plot at.
#[must_use]
pub fn plot_extent(composed: &Composed, plot: usize) -> Option<Rect> {
    let handle = composed.plots.get(plot)?;
    let mut extent = handle.rect;
    for band in [handle.legend_right, handle.legend_below]
        .into_iter()
        .flatten()
    {
        let right = (extent.x + extent.width).max(band.x + band.width);
        let bottom = (extent.y + extent.height).max(band.y + band.height);
        extent.x = extent.x.min(band.x);
        extent.y = extent.y.min(band.y);
        extent.width = right - extent.x;
        extent.height = bottom - extent.y;
    }
    Some(extent)
}

/// Draw plot `plot` of `composed` as a picture at `scale` device pixels per
/// logical point: the export of the whole page, cut to [`plot_extent`].
///
/// # Errors
/// Returns a message when the composition has no such plot, or on GPU failure.
pub fn render_plot(
    composed: &Composed,
    plot: usize,
    scale: f32,
) -> Result<image::RgbaImage, String> {
    let extent =
        plot_extent(composed, plot).ok_or_else(|| "there is no chart to draw".to_owned())?;
    let page = render_export(composed, scale)?;
    let s = f64::from(scale);
    let (w, h) = page.dimensions();
    let x0 = ((extent.x * s).round().max(0.0) as u32).min(w);
    let y0 = ((extent.y * s).round().max(0.0) as u32).min(h);
    let x1 = (((extent.x + extent.width) * s).round().max(0.0) as u32).min(w);
    let y1 = (((extent.y + extent.height) * s).round().max(0.0) as u32).min(h);
    if x1 <= x0 || y1 <= y0 {
        return Err("the chart has no area to draw".to_owned());
    }
    Ok(image::imageops::crop_imm(&page, x0, y0, x1 - x0, y1 - y0).to_image())
}

/// The picture as egui hands it to the clipboard.
#[must_use]
pub fn clipboard_image(picture: &image::RgbaImage) -> egui::ColorImage {
    let (w, h) = picture.dimensions();
    egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], picture.as_raw())
}

/// The name a saved picture is offered under, without its extension: `base`,
/// made safe for a file name, and the plot's place on the page when the page
/// has more than one.
#[must_use]
pub fn file_stem(base: &str, plot: usize, plots: usize) -> String {
    let mut stem = String::new();
    for c in base.trim().chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            stem.extend(c.to_lowercase());
        } else if !stem.ends_with('-') && !stem.is_empty() {
            stem.push('-');
        }
    }
    let stem = stem.trim_end_matches('-');
    let stem = if stem.is_empty() { "chart" } else { stem };
    if plots > 1 {
        format!("{stem}-{}", plot + 1)
    } else {
        stem.to_owned()
    }
}

/// Where a Protocol's exported pictures go: `exports/` in its folder, where
/// `brightfield export png` writes them.
#[must_use]
pub fn exports_dir(protocol_dir: &Path) -> PathBuf {
    protocol_dir.join("exports")
}

/// Ask where to save a picture, blocking until the user chooses or cancels;
/// `None` when they cancel. Offers `name` in `dir`, which is made first so the
/// dialog can open there.
///
/// The one call here no headless test drives, as `data_file::pick` is the one
/// in its module: the window raises it where `dialogs_allowed` says a person
/// is in front of it.
#[must_use]
pub fn ask_save_path(dir: Option<&Path>, name: &str) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Export chart as a picture")
        .add_filter("PNG image", &["png"])
        .set_file_name(name);
    if let Some(dir) = dir {
        if std::fs::create_dir_all(dir).is_ok() {
            dialog = dialog.set_directory(dir);
        }
    }
    dialog.save_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_stem_is_the_name_made_safe_and_numbered_on_a_page_of_several() {
        assert_eq!(file_stem("CO2 per capita", 0, 1), "co2-per-capita");
        assert_eq!(
            file_stem("Sales / region (2026)", 2, 3),
            "sales-region-2026-3"
        );
        assert_eq!(file_stem("  ", 0, 1), "chart");
        assert_eq!(file_stem("penguin_mass", 0, 2), "penguin_mass-1");
    }
}
