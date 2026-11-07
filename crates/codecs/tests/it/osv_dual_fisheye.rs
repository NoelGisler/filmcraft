//! DJI Osmo 360 `.OSV`, phase 2 (docs/osv-notes.md): a dual-fisheye clip plays both lenses side by
//! side in one `2W`×`H` frame, lens 0 (track `v:0`) left, lens 1 (`v:1`) right.
//!
//! Oracle: `ffmpeg -i <file> -filter_complex "[0:v:0][0:v:1]hstack"`, compared bit-exactly like
//! the other HEVC oracle tests (`mp4_oracle.rs`). ffmpeg is an external oracle and fixture
//! generator only. Stand-ins are made with libx265 (8-bit and Main 10, with GOPs that differ
//! between the lenses); the camera file is used as well when one is available
//! (`$FILMCRAFT_OSV_SAMPLE` or `target/fixtures/osv/sample.osv`, never committed).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use filmcraft_frame::{PixelData, VideoFrame};
use filmcraft_media::{FrameRequest, MediaSource, SphericalLayout};
use filmcraft_time::Tick;

use crate::common::*;

/// Two lenses of different content (a swapped or repeated lens shows), 2 s at 25 fps, with
/// B-frames and short GOPs so that seeking re-enters the middle of a GOP. `ten_bit`: Main 10, and
/// lens 1 gets a different GOP length than lens 0 (each lens seeks to its own sync sample).
fn stand_in(ff: &Path, name: &str, w: u32, h: u32, ten_bit: bool) -> PathBuf {
    stand_in_with(ff, name, w, h, ten_bit, ",noise=alls=12:allf=t")
}

/// [`stand_in`] with `lens1_filter` after lens 1's test pattern (its noise makes it much costlier
/// to decode than lens 0: the speed measurement leaves it out, two lenses of one camera cost about
/// the same).
fn stand_in_with(ff: &Path, name: &str, w: u32, h: u32, ten_bit: bool, lens1_filter: &str) -> PathBuf {
    let size = format!("{w}x{h}");
    let (pix, gop1) = if ten_bit { ("yuv420p10le", "keyint=9:min-keyint=9") } else { ("yuv420p", "keyint=12:min-keyint=12") };
    let p0 = "log-level=error:bframes=3:keyint=12:min-keyint=12:scenecut=0".to_string();
    let p1 = format!("log-level=error:bframes=3:{gop1}:scenecut=0");
    let (src0, src1) = (format!("testsrc=size={size}:rate=25:d=2"), format!("testsrc2=size={size}:rate=25:d=2{lens1_filter}"));
    #[rustfmt::skip]
    let args = [
        "-f", "lavfi", "-i", &src0,
        "-f", "lavfi", "-i", &src1,
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:d=2",
        "-map", "0:v", "-map", "1:v", "-map", "2:a",
        "-c:v", "libx265", "-x265-params:v:0", &p0, "-x265-params:v:1", &p1,
        "-pix_fmt", pix, "-tag:v", "hvc1", "-c:a", "aac", "-f", "mp4",
    ];
    fixture(ff, name, &args).unwrap_or_else(|| panic!("could not write {name} (needs ffmpeg with libx265)"))
}

/// The frames ffmpeg makes of `file` with `filter` (one video output), as raw `pix_fmt` frames of
/// `frame_bytes` each, handed to `each` one at a time (a camera file's frames are large).
fn ffmpeg_each_frame(ff: &Path, file: &Path, filter: &[&str], pix_fmt: &str, frame_bytes: usize, mut each: impl FnMut(usize, &[u8])) -> usize {
    let mut child = Command::new(ff)
        .args(["-v", "error", "-i", file.to_str().unwrap()])
        .args(filter)
        .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", pix_fmt, "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run ffmpeg");
    let mut out = child.stdout.take().unwrap();
    let mut buf = vec![0u8; frame_bytes];
    let mut n = 0;
    while out.read_exact(&mut buf).is_ok() {
        each(n, &buf);
        n += 1;
    }
    assert!(child.wait().unwrap().success(), "ffmpeg failed");
    n
}

const HSTACK: [&str; 2] = ["-filter_complex", "[0:v:0][0:v:1]hstack"];

fn bits(f: &VideoFrame) -> u32 {
    match &f.data {
        PixelData::Yuv8 { .. } => 8,
        PixelData::Yuv16 { bits, .. } => *bits,
        _ => panic!("expected a Y'CbCr frame"),
    }
}

fn open(path: &Path) -> std::sync::Arc<dyn MediaSource> {
    filmcraft_codecs::open_bytes(path.file_name().unwrap().to_str().unwrap(), bytes(path)).unwrap()
}

fn frame_at(src: &dyn MediaSource, t: Tick) -> std::sync::Arc<VideoFrame> {
    src.video_frame(FrameRequest::full(t)).unwrap()
}

/// Every frame, played from the start, equals ffmpeg's hstack of the two lens tracks; returns the
/// frames' planes for the seek test.
fn assert_matches_hstack(ff: &Path, path: &Path, keep: impl Fn(usize) -> bool) -> Vec<(usize, Vec<Vec<u16>>)> {
    let src = open(path);
    let info = src.info();
    assert_eq!(info.spherical, Some(SphericalLayout::DualFisheye));
    let v = info.video.clone().unwrap();
    let first = frame_at(&*src, Tick::ZERO);
    let (w, h) = (first.width as usize, first.height as usize);
    assert_eq!((first.width, first.height), (v.width, v.height), "the reported size is the frame's");
    let depth = bits(&first);
    let (pix, bps) = if depth > 8 { ("yuv420p10le", 2) } else { ("yuv420p", 1) };
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut kept = Vec::new();
    let n = ffmpeg_each_frame(ff, path, &HSTACK, pix, (w * h + 2 * cw * ch) * bps, |i, raw| {
        let f = frame_at(&*src, v.frame_rate.tick_of(i as i64));
        assert_eq!((f.width as usize, f.height as usize), (w, h), "frame {i}: size");
        assert_eq!(bits(&f), depth, "frame {i}: bit depth");
        let ours = planes(&f);
        assert!(
            ours == raw_planes(raw, w, h, cw, ch, bps),
            "frame {i} differs from ffmpeg's hstack (max diff {})",
            max_diff(&ours, &raw_planes(raw, w, h, cw, ch, bps))
        );
        if keep(i) {
            kept.push((i, ours));
        }
    });
    assert!(n > 0, "ffmpeg made no frames");
    kept
}

#[test]
fn dual_fisheye_8_bit_matches_ffmpeg_hstack() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let f = stand_in(&ff, "osv_dual_8bit.osv", 96, 64, false);
    let src = open(&f);
    let v = src.info().video.clone().unwrap();
    assert_eq!((v.width, v.height), (192, 64), "two 96×64 lenses side by side");
    assert_matches_hstack(&ff, &f, |_| false);
}

#[test]
fn dual_fisheye_main_10_keeps_10_bit_and_matches_ffmpeg_hstack() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let f = stand_in(&ff, "osv_dual_main10.osv", 96, 64, true);
    let src = open(&f);
    assert_eq!(bits(&frame_at(&*src, Tick::ZERO)), 10, "Main 10 stays 10-bit");
    assert_matches_hstack(&ff, &f, |_| false);
}

/// 1.5 s: frame 37 at 25 fps (1.48 s; frame 38 starts at 1.52 s).
const SEEK_TO: Tick = Tick(filmcraft_time::TICKS_PER_SECOND * 3 / 2);

/// The frame at 1.5 s reached by seeking (forward from the start, back from the end, and from
/// another GOP) equals the one played linearly up to it (and ffmpeg's).
fn assert_seek_matches_linear(ff: &Path, path: &Path) {
    let rate = open(path).info().video.clone().unwrap().frame_rate;
    let want = rate.frame_at(SEEK_TO);
    let linear = assert_matches_hstack(ff, path, |i| i as i64 == want);
    let [(_, linear)] = linear.as_slice() else { panic!("no frame {want} in ffmpeg's output") };
    let dur = open(path).info().duration;
    for (what, before) in [("cold seek", None), ("after the last frame", Some(dur - rate.frame_duration())), ("after frame 3", Some(rate.tick_of(3)))] {
        let src = open(path);
        if let Some(t) = before {
            frame_at(&*src, t);
        }
        let f = frame_at(&*src, SEEK_TO);
        assert!(planes(&f) == *linear, "{what}: the frame at 1.5 s differs from linear playback");
    }
}

#[test]
fn dual_fisheye_seek_to_1_5_s_equals_linear_playback() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    for (name, ten) in [("osv_dual_8bit.osv", false), ("osv_dual_main10.osv", true)] {
        assert_seek_matches_linear(&ff, &stand_in(&ff, name, 96, 64, ten));
    }
}

/// Normal MP4 clips don't change: the same two-track file named `.mp4` plays track `v:0` alone,
/// exactly as ffmpeg decodes it.
#[test]
fn the_same_file_as_mp4_still_plays_its_first_track_alone() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let osv = stand_in(&ff, "osv_dual_8bit.osv", 96, 64, false);
    let mp4 = dir().join("osv_dual_8bit_as.mp4");
    std::fs::copy(&osv, &mp4).unwrap();
    let src = open(&mp4);
    assert_eq!(src.info().spherical, None);
    let v = src.info().video.clone().unwrap();
    assert_eq!((v.width, v.height), (96, 64));
    let (cw, ch) = (48, 32);
    let n = ffmpeg_each_frame(&ff, &mp4, &["-map", "0:v:0"], "yuv420p", 96 * 64 + 2 * cw * ch, |i, raw| {
        let f = frame_at(&*src, v.frame_rate.tick_of(i as i64));
        assert!(planes(&f) == raw_planes(raw, 96, 64, cw, ch, 1), "frame {i}");
    });
    assert_eq!(n, 50);
}

/// Hardware decoders handed out so far by [`one_session_factory`].
static SESSIONS: AtomicUsize = AtomicUsize::new(0);
/// A lens width no other test uses: only the fixture below reaches the fake hardware factory.
const ONE_SESSION_WIDTH: u16 = 136;

/// A stand-in hardware decoder factory with a single session: the first decoder made for the
/// fixture's HEVC tracks is "hardware" (our software decoder, so the output is the same), every
/// further one fails as a GPU out of decode sessions does.
fn one_session_factory(entry: &filmcraft_isobmff::SampleEntry) -> Option<filmcraft_codecs::Result<Box<dyn filmcraft_codecs::VideoDecoder>>> {
    if entry.video.as_ref().map(|v| v.width) != Some(ONE_SESSION_WIDTH) {
        return None;
    }
    Some(if SESSIONS.fetch_add(1, Ordering::SeqCst) == 0 {
        filmcraft_codecs::software_video_decoder(entry)
    } else {
        Err(filmcraft_codecs::CodecError::Unsupported("no hardware decode session left".into()))
    })
}

#[test]
fn a_lens_without_a_hardware_session_decodes_in_software() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let f = stand_in(&ff, "osv_dual_one_session.osv", u32::from(ONE_SESSION_WIDTH), 64, true);
    filmcraft_codecs::register_video_decoder(one_session_factory);
    assert_matches_hstack(&ff, &f, |_| false);
    assert!(SESSIONS.load(Ordering::SeqCst) >= 2, "both lenses asked the hardware factory for a decoder");
}

fn real_sample() -> Option<PathBuf> {
    let p = std::env::var_os("FILMCRAFT_OSV_SAMPLE")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| filmcraft_testkit::fixtures::fixtures_root().join("osv").join("sample.osv"));
    p.is_file().then_some(p)
}

#[test]
fn real_osv_sample_matches_ffmpeg_hstack_and_seeks() {
    let Some(path) = real_sample() else {
        eprintln!("skipped: no camera .osv (set FILMCRAFT_OSV_SAMPLE or put one at target/fixtures/osv/sample.osv)");
        return;
    };
    let ff = filmcraft_testkit::require_ffmpeg!();
    assert_seek_matches_linear(&ff, &path);
}

/// Decoding speed of the side-by-side frame, played from the start (`--ignored`; the result goes
/// into docs/osv-notes.md), beside each lens decoded alone. The camera file when there is one, else
/// a 1920×1920-per-lens Main 10 stand-in.
#[test]
#[ignore = "measurement, run by hand"]
fn dual_fisheye_playback_fps() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    let path = real_sample().unwrap_or_else(|| stand_in_with(&ff, "osv_dual_perf_1920_clean.osv", 1920, 1920, true, ""));
    // one-lens baselines: each lens track copied into an .mp4 of its own
    let lens = |k: usize| {
        let out = dir().join(format!("osv_dual_perf_lens{k}.mp4"));
        ffmpeg_out(&ff, &["-y", "-i", path.to_str().unwrap(), "-map", &format!("0:v:{k}"), "-c", "copy", out.to_str().unwrap()]);
        out
    };
    let (l0, l1) = (lens(0), lens(1));
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    for (what, p) in [("both lenses", &path), ("lens 0 alone", &l0), ("lens 1 alone", &l1)] {
        let src = open(p);
        let v = src.info().video.clone().unwrap();
        let n = v.frame_rate.frame_at(src.info().duration).max(1);
        let (t0, g0) = (std::time::Instant::now(), filmcraft_codecs::gop_stats());
        for i in 0..n {
            frame_at(&*src, v.frame_rate.tick_of(i));
        }
        let secs = t0.elapsed().as_secs_f64();
        let g = filmcraft_codecs::gop_stats() - g0;
        eprintln!("  decoded {} samples, {} seeks, {} evicted", g.decoded, g.seeks, g.evicted);
        eprintln!(
            "{what} ({}): {}x{} {:?}, {n} frames in {secs:.2} s = {:.1} fps ({threads} threads)",
            path.file_name().unwrap().to_string_lossy(),
            v.width,
            v.height,
            v.pixel_format,
            n as f64 / secs
        );
    }
}
