//! DJI Osmo 360 `.OSV` import, phase 1 (docs/osv-notes.md); the side-by-side frames of phase 2 are
//! checked in `osv_dual_fisheye.rs`.
//!
//! - An ffmpeg-made stand-in (two real HEVC tracks, one per "lens", plus AAC) checks the whole open
//!   path and that lens 0 decodes. ffmpeg is a fixture generator only.
//! - A real camera file is used when one is available: `$FILMCRAFT_OSV_SAMPLE`, else
//!   `<fixtures root>/osv/sample.osv` (`target/fixtures/osv/sample.osv`). Camera files are never
//!   committed (AGENTS.md); the test is skipped without one.

use std::path::PathBuf;

use filmcraft_media::{FrameRequest, SphericalLayout};
use filmcraft_time::Tick;

use crate::common::*;

#[test]
fn ffmpeg_dual_hevc_osv_imports_as_dual_fisheye_and_lens_0_decodes() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    #[rustfmt::skip]
    let args = [
        "-f", "lavfi", "-i", "testsrc=size=128x128:rate=25:d=1",
        "-f", "lavfi", "-i", "testsrc2=size=128x128:rate=25:d=1",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:d=1",
        "-map", "0:v", "-map", "1:v", "-map", "2:a",
        "-c:v", "libx265", "-x265-params", "log-level=error", "-pix_fmt", "yuv420p", "-tag:v", "hvc1",
        "-c:a", "aac", "-f", "mp4",
    ];
    let Some(f) = fixture(&ff, "dual_hevc.osv", &args) else { panic!("could not write the dual-HEVC .osv fixture (needs ffmpeg with libx265)") };
    let src = filmcraft_codecs::open_bytes("dual_hevc.osv", bytes(&f)).unwrap();
    let info = src.info();
    assert_eq!(info.spherical, Some(SphericalLayout::DualFisheye));
    let v = info.video.as_ref().unwrap();
    // phase 2: both 128×128 lenses side by side (osv_dual_fisheye.rs checks the pixels)
    assert_eq!((v.width, v.height, v.codec.as_str()), (256, 128, "HEVC"));
    assert_eq!(info.audio_streams.len(), 1);
    let frame = src.video_frame(FrameRequest::full(Tick::ZERO)).unwrap();
    assert_eq!((frame.width, frame.height), (256, 128));
}

fn real_sample() -> Option<PathBuf> {
    let p = std::env::var_os("FILMCRAFT_OSV_SAMPLE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| filmcraft_testkit::fixtures::fixtures_root().join("osv").join("sample.osv"));
    p.is_file().then_some(p)
}

#[test]
fn real_osv_sample_imports_as_dual_fisheye() {
    let Some(path) = real_sample() else {
        eprintln!("skipped: no camera .osv (set FILMCRAFT_OSV_SAMPLE or put one at target/fixtures/osv/sample.osv)");
        return;
    };
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let src = filmcraft_codecs::open_bytes(&name, bytes(&path)).unwrap();
    let info = src.info();
    eprintln!("{name}: {info:?}");
    assert_eq!(info.spherical, Some(SphericalLayout::DualFisheye));
    assert_eq!(info.video.as_ref().map(|v| v.codec.as_str()), Some("HEVC"));
    assert!(info.has_audio(), "the camera records audio");
    // the fixture is 3 s long
    let secs = info.duration.seconds();
    assert!((2.5..3.5).contains(&secs), "duration {secs} s");
    // both lenses decode, side by side (phase 2)
    let frame = src.video_frame(FrameRequest::full(Tick::ZERO)).unwrap();
    let v = info.video.as_ref().unwrap();
    assert_eq!((frame.width, frame.height), (v.width, v.height));
}
