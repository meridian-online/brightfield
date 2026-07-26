//! The half of the sampling notice that can be machine-checked: that it
//! survives a chart-only export.
//!
//! `capture_vello_only` rasterises the composed Vello scene and never
//! constructs an egui context, so everything the shell draws — the top bar, the
//! margin legend, any banner anyone might be tempted to add — is absent from
//! that PNG **by construction**. That is the mechanical meaning of "survives
//! being cropped out of a screenshot", and it is exactly why the notice is
//! drawn into the plot's own scene rather than into chrome.
//!
//! What is left for a human eye is the other half — whether a sampled render
//! reads as sampled without reading the words. Nothing here claims to test
//! that.

use std::path::PathBuf;

use brightfield_render::sample_notice::NOTICE_BAND;
use brightfield_shell::capture::capture_vello_only;
use brightfield_shell::pipeline::compose_spec_sampled;
use brightfield_sql::ir::SampleRate;

/// Small enough to render fast, dense enough to be a real scatter.
const SPEC: &str = "data:
  points:
    query: |
      SELECT (i * 7919 % 1009) / 10.0 AS a, (i * 104729 % 1013) / 10.0 AS b
      FROM range(4096) AS t(i)
plot:
  - mark: dot
    data: { from: points }
    x: a
    y: b
width: 400
height: 300
";

const W: u32 = 400;
const H: u32 = 300;

fn write_spec(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("sampled-export.yaml");
    std::fs::write(&p, SPEC).expect("write spec");
    p
}

/// Count pixels that are not the near-white page/surface, in the bottom
/// `NOTICE_BAND` logical rows — the band the notice reserves.
fn ink_in_band(png: &std::path::Path) -> u64 {
    let img = image::open(png).expect("open png").to_rgba8();
    let band_top = H - NOTICE_BAND.ceil() as u32;
    let mut ink = 0u64;
    for y in band_top..H {
        for x in 0..W {
            let p = img.get_pixel(x, y).0;
            // The page and surface tokens are both above 0xf8 on every
            // channel; anything appreciably darker is ink someone drew.
            if p[0] < 0xf0 || p[1] < 0xf0 || p[2] < 0xf0 {
                ink += 1;
            }
        }
    }
    ink
}

#[test]
fn the_sampling_notice_is_in_the_chart_only_export() {
    let dir = std::env::temp_dir().join(format!("bf-sampled-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let spec = write_spec(&dir);

    let complete = compose_spec_sampled(spec.to_str().unwrap(), None).expect("compose complete");
    let (cw, ch) = (complete.width, complete.height);
    let rate = SampleRate::from_modulus(8).expect("power of two");
    let sampled =
        compose_spec_sampled(spec.to_str().unwrap(), Some(rate)).expect("compose sampled");

    // The fact reached the plot handle, and only on the sampled composition.
    assert!(
        complete.plots.iter().all(|p| p.sample.is_none()),
        "an unsampled composition must carry no sampling fact"
    );
    let fact = sampled.plots[0]
        .sample
        .expect("the sampled composition's plot must carry its fact");
    let sampled_size = (sampled.width, sampled.height);
    assert_eq!(fact.of, 4096, "`of` is the unsampled count, measured");
    assert!(
        fact.drawn > 0 && fact.drawn < fact.of,
        "a 1-in-8 sample of 4096 rows drew {} — expected some but not all",
        fact.drawn
    );

    // Same canvas either way: the band is taken out of the plot's margin, not
    // added to the image, so the two PNGs are directly comparable.
    assert_eq!((cw, ch), (W, H));
    assert_eq!(sampled_size, (W, H));

    let complete_png = dir.join("complete.png");
    let sampled_png = dir.join("sampled.png");
    capture_vello_only(complete, 1.0, &complete_png).expect("capture complete");
    capture_vello_only(sampled, 1.0, &sampled_png).expect("capture sampled");

    let complete_ink = ink_in_band(&complete_png);
    let sampled_ink = ink_in_band(&sampled_png);

    assert!(
        sampled_ink > 400,
        "the sampled export's bottom band held {sampled_ink} inked pixels — the hatch and \
         label should be plainly there. If this is near zero the notice did not survive the \
         chart-only export, which is the whole reason it is not a banner."
    );
    assert!(
        sampled_ink > complete_ink * 3,
        "the band is supposed to be the DIFFERENCE between the two exports: sampled held \
         {sampled_ink} inked pixels there, complete held {complete_ink}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
