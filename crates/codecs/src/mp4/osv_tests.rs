//! DJI Osmo 360 `.OSV` import, phase 1 (docs/osv-notes.md): an ISO BMFF file with one HEVC video
//! track per fisheye lens, an audio track and a DJI metadata track. The files here are built in
//! memory with the muxer (no media is committed); the metadata track's fourcc is a stand-in, since
//! the demuxer treats every unknown handler and sample entry alike.

use std::sync::Arc;

use filmcraft_isobmff::{Brand, CodecConfig, FourCc, HevcConfig, Mp4Writer, PcmConfig, SampleEntry, TrackConfig, TrackKind, WriteSample, WriterOptions};
use filmcraft_media::{MediaKind, MediaSource, SphericalLayout};

use super::Mp4Source;

/// A stand-in for the DJI metadata sample entry (the real fourcc is recorded in docs/osv-notes.md
/// once confirmed against a camera file).
const META_FOURCC: [u8; 4] = *b"xmet";

#[derive(Clone)]
struct Spec {
    /// Picture size of each video track (one per lens).
    lenses: Vec<(u16, u16)>,
    audio: bool,
    /// Payload of each metadata sample (`None`: no metadata track).
    meta: Option<Vec<Vec<u8>>>,
}

impl Spec {
    fn osv() -> Self {
        Spec { lenses: vec![(64, 64), (64, 64)], audio: true, meta: Some(vec![vec![0xD5; 40]; 3]) }
    }
}

fn hevc_entry(w: u16, h: u16) -> SampleEntry {
    let cfg = HevcConfig { general_profile_idc: 1, chroma_format_idc: 1, bit_depth_luma: 8, ..Default::default() };
    SampleEntry::video(FourCc(*b"hvc1"), CodecConfig::Hevc(cfg), w, h)
}

fn meta_entry() -> SampleEntry {
    SampleEntry {
        format: FourCc(META_FOURCC),
        data_reference_index: 1,
        // long enough to also parse as a visual sample entry (78 bytes): a shorter one under a
        // `vide` handler fails the whole file in the demuxer (docs/osv-notes.md, open points)
        codec: CodecConfig::Unknown { fourcc: FourCc(META_FOURCC), raw: vec![0; 86] },
        video: None,
        audio: None,
        bitrate: None,
    }
}

/// Three 25 fps frames per lens (placeholder HEVC payloads: opening a file reads no picture data),
/// 0.12 s of PCM and one metadata sample per frame, in that track order.
fn build(spec: &Spec) -> Arc<[u8]> {
    let mut mux = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    for &(w, h) in &spec.lenses {
        let t = mux.add_track(TrackConfig::new(hevc_entry(w, h), 25)).unwrap();
        for i in 0..3 {
            mux.write_sample(t, WriteSample { data: &[0, 0, 0, 2, 0x26, 0x01], duration: 1, composition_offset: 0, is_sync: i == 0 }).unwrap();
        }
    }
    if spec.audio {
        let entry = SampleEntry::pcm(PcmConfig { bits: 16, float: false, big_endian: false, signed: true, channels: 2, sample_rate: 48_000.0 });
        let t = mux.add_track(TrackConfig::new(entry, 48_000)).unwrap();
        let data = vec![0u8; 5760 * 4];
        mux.write_sample(t, WriteSample { data: &data, duration: 5760, composition_offset: 0, is_sync: true }).unwrap();
    }
    if let Some(samples) = &spec.meta {
        let t = mux.add_track(TrackConfig::new(meta_entry(), 25)).unwrap();
        for s in samples {
            mux.write_sample(t, WriteSample { data: s, duration: 1, composition_offset: 0, is_sync: true }).unwrap();
        }
    }
    mux.finish().unwrap().into_inner().into()
}

#[test]
fn osv_with_two_lenses_audio_and_metadata_is_dual_fisheye() {
    let bytes = build(&Spec::osv());
    // the metadata track is in the file, as a track of no kind we play
    let file = filmcraft_isobmff::open(&*bytes).unwrap();
    assert_eq!(file.tracks.iter().map(|t| t.kind).collect::<Vec<_>>(), [TrackKind::Video, TrackKind::Video, TrackKind::Audio, TrackKind::Other]);

    let src = Mp4Source::open("CAM_20261101_0001.OSV", bytes).unwrap();
    let info = src.info();
    assert_eq!(info.kind, MediaKind::Movie);
    assert_eq!(info.spherical, Some(SphericalLayout::DualFisheye));
    assert_eq!(src.fisheye_lens_tracks(), Some([0, 1]));
    let v = info.video.as_ref().unwrap();
    assert_eq!((v.width, v.height, v.codec.as_str()), (64, 64, "HEVC"));
    assert_eq!(info.audio_streams.len(), 1, "the metadata track is not an audio stream");
    assert_eq!(info.audio().map(|a| (a.sample_rate, a.channels)), Some((48_000, 2)));
    assert_eq!(info.duration, filmcraft_time::Tick::from_units(3, 25));
    assert_eq!(SphericalLayout::DualFisheye.label(), "dual-fisheye 360");
}

/// The importer reaches the MP4 source through the registered openers; nothing about `.osv`
/// needs its own opener.
#[test]
fn osv_opens_through_the_import_openers() {
    let src = crate::open_bytes("clip.osv", build(&Spec::osv())).unwrap();
    assert_eq!(src.info().spherical, Some(SphericalLayout::DualFisheye));
    assert!(filmcraft_media::is_importable(std::path::Path::new("clip.osv")));
}

/// The same tracks in an ordinary `.mp4` stay flat video (multi-angle or picture-in-picture files
/// exist): only an `.osv` name says the two tracks are lenses.
#[test]
fn two_video_tracks_in_an_mp4_are_not_spherical() {
    let src = Mp4Source::open("two_angles.mp4", build(&Spec::osv())).unwrap();
    assert_eq!(src.info().spherical, None);
    assert_eq!(src.fisheye_lens_tracks(), None);
    assert!(src.info().has_video() && src.info().has_audio());
}

/// Odd `.osv` files open as flat video from their first lens instead of failing the import.
#[test]
fn osv_without_a_matching_lens_pair_opens_as_flat_video() {
    let cases = [
        ("one lens", Spec { lenses: vec![(64, 64)], ..Spec::osv() }),
        ("lens sizes differ", Spec { lenses: vec![(64, 64), (32, 64)], ..Spec::osv() }),
        ("three video tracks", Spec { lenses: vec![(64, 64); 3], ..Spec::osv() }),
    ];
    for (what, spec) in cases {
        let src = Mp4Source::open("odd.osv", build(&spec)).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(src.info().spherical, None, "{what}");
        assert_eq!(src.info().video.as_ref().map(|v| v.width), Some(64), "{what}: lens 0 is the picture");
    }
}

/// The metadata track is skipped whatever it holds: missing, empty, or full of junk.
#[test]
fn metadata_track_is_skipped_whatever_it_holds() {
    let cases = [
        ("no metadata track", None),
        ("metadata track with no samples", Some(Vec::new())),
        ("empty samples", Some(vec![Vec::new(); 3])),
        ("junk samples", Some(vec![vec![0xFF; 4096], vec![0x00; 1], b"not dji at all".to_vec()])),
    ];
    for (what, meta) in cases {
        let src = Mp4Source::open("meta.osv", build(&Spec { meta, ..Spec::osv() })).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(src.info().spherical, Some(SphericalLayout::DualFisheye), "{what}");
        assert_eq!(src.info().audio_streams.len(), 1, "{what}");
    }
    // metadata declared with a video handler: never taken for a lens
    let mut b = build(&Spec::osv()).to_vec();
    let at = b.windows(8).rposition(|w| w == b"hdlr\0\0\0\0").unwrap() + 12;
    assert_eq!(&b[at..at + 4], b"meta");
    b[at..at + 4].copy_from_slice(b"vide");
    let src = Mp4Source::open("vide_meta.osv", b.into()).unwrap();
    assert_eq!(src.info().spherical, Some(SphericalLayout::DualFisheye));
    assert_eq!(src.fisheye_lens_tracks(), Some([0, 1]));
    // no audio either: still a 360 movie
    let src = Mp4Source::open("silent.osv", build(&Spec { audio: false, ..Spec::osv() })).unwrap();
    assert_eq!((src.info().spherical, src.info().has_audio()), (Some(SphericalLayout::DualFisheye), false));
}

/// Truncated and bit-flipped `.osv` files open or fail with an error, never a panic (AGENTS.md §0.8).
#[test]
fn damaged_osv_never_panics() {
    let good = build(&Spec::osv());
    let try_open = |b: Vec<u8>| std::panic::catch_unwind(|| Mp4Source::open("damaged.osv", b.into()).map(|s| s.info().spherical)).is_ok();
    for cut in (0..good.len()).step_by(7) {
        assert!(try_open(good[..cut].to_vec()), "panicked when cut at {cut}");
    }
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    for round in 0..400 {
        let mut b = good.to_vec();
        for _ in 0..1 + round % 4 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let at = (rng as usize) % b.len();
            b[at] ^= 1 << ((rng >> 32) % 8);
        }
        assert!(try_open(b), "panicked in bit-flip round {round}");
    }
}
