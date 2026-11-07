# DJI Osmo 360 `.OSV` import

Status: **phase 1 done**: `.osv` files import as clips marked *dual-fisheye 360*. Nothing is
stitched yet: the monitors show lens 0 as a plain fisheye picture.

## The format (as far as phase 1 relies on it)

An `.OSV` file is an ISO BMFF (MP4) file with:

- two HEVC video tracks, one per fisheye lens, with the same codec and picture size;
- an audio track;
- a DJI metadata track (camera motion and lens data, not parsed in phase 1).

Source: the project's own `ffprobe` inspection of a camera file (`osv_probe.txt`, kept outside the
repository with the camera file). Clean-room: no code or documentation from OpenOSV, Gyroflow or
other OSV tools was read (AGENTS.md §2). ffprobe/ffmpeg are used only as external oracles and
fixture generators.

## Decisions (phase 1)

1. **No container work.** `filmcraft-isobmff` re-exports `craft_media::isobmff` (craft-libs). Its
   demuxer already returns every track: an unknown handler is `TrackKind::Other`, an unknown sample
   entry is `CodecConfig::Unknown`. The metadata track is therefore skipped simply because no
   FilmCraft code looks at `Other` tracks. Nothing changed in craft-libs.
2. **Extension.** `"osv"` is in `filmcraft_media::VIDEO_EXTENSIONS`, so it can be imported, filtered
   in the media browser and offered in file dialogs (case-insensitive, like every extension). The file
   opens through the normal MP4 opener (`filmcraft_codecs::mp4`), whose sniffer recognises the
   `ftyp` box. No OSV-specific opener is needed.
3. **Marking.** New `MediaInfo::spherical: Option<SphericalLayout>` with one variant,
   `SphericalLayout::DualFisheye` (label: "dual-fisheye 360"). The field is
   `#[serde(default, skip_serializing_if = "Option::is_none")]`: older project files load as flat
   video, and flat clips write no key. Every other source sets `None`.
4. **Detection rule.** `Mp4Source` marks a file as dual-fisheye only when **all** of these hold:
   - the name ends in `.osv` (any case);
   - exactly two playable video tracks have a known codec (a track with
     `CodecConfig::Unknown` is never a lens, even under a `vide` handler);
   - both have the same codec, the same fourcc and the same non-zero picture size.

   Otherwise the file still opens, as flat video from its first video track. This covers one lens,
   lenses that differ, or a third video track. The name is used because two equal video tracks also
   occur in ordinary MP4 files (multi-angle recordings). At most 8 video tracks are examined, since
   the track count comes from the file.
5. **What plays.** `MediaInfo::video` describes lens 0 (the first lens track in file order) and only
   lens 0 is decoded. `Mp4Source::fisheye_lens_tracks()` returns both track indices for phase 2.
   Audio works as for any MP4: every playable audio track is a stream.
6. **Tests** (no media committed):
   - `crates/codecs/src/mp4/osv_tests.rs` builds files in memory with the muxer: two HEVC lens
     tracks, PCM and a metadata track. Cases:
     - the file is marked dual-fisheye;
     - the import openers reach it;
     - the same file named `.mp4` stays flat;
     - one lens, unequal lenses or three video tracks give flat video;
     - a metadata track that is missing, empty, holds junk, or is declared as video;
     - no audio;
     - truncation and bit-flip fuzzing under `catch_unwind`.
   - `crates/codecs/tests/it/osv_import.rs`:
     - an ffmpeg-made stand-in (two real libx265 tracks plus AAC, named `.osv`) checks the
       marking and that lens 0 decodes;
     - an optional camera-file test reads `$FILMCRAFT_OSV_SAMPLE` or
       `target/fixtures/osv/sample.osv` and is skipped when neither exists.
   - `crates/media` checks the extension and the `MediaInfo` serde compatibility.

## Open points for phase 2

- **Camera file in CI.** The 3 s `sample.osv` cannot be committed (AGENTS.md: no large binary
  fixtures). Put it in a corpus repository (as `storytold/photocraft-corpus` does for PSDs, pinned
  by commit and sha256) and fetch it in `cargo xtask corpus`. Until then, run the camera test locally
  with `FILMCRAFT_OSV_SAMPLE=/path/to/sample.osv cargo test -p filmcraft-codecs --test it osv`.
- **Check the probe facts against the test.** Use `osv_probe.txt` to confirm:
  - the metadata track's handler type and sample-entry fourcc (the synthetic test uses a stand-in,
    `xmet`);
  - the `ftyp` major and compatible brands;
  - track order, picture size and frame rate per lens;
  - the edit lists.

  If the brand is DJI-specific, detect by brand as well as by name, so a renamed file is still
  recognised.
- **Demuxer robustness (craft-libs).** A short or non-visual sample entry under a `vide` handler
  makes `craft_media::isobmff::open` reject the **whole file** ("truncated data while reading sample
  entry"). The synthetic tests found this and work around it. If a DJI file (or a
  damaged one) has a metadata track like that, the import fails. Fix in craft-libs: parse an unknown
  sample entry as `Unknown` instead of failing the movie.
- **Decode both lenses.** Phase 1 has one `GopCache` for lens 0. Phase 2 needs a second decoder and
  cache for lens 1, with frame-accurate pairing of the two tracks (same timestamps; check the edit
  lists) and cache budgeting for two HEVC streams.
- **Projection.** Dual-fisheye to equirectangular (or a direct viewport) render step in
  `crates/render`/`gpu`. This needs lens field of view, the image circle centre and radius, and
  per-lens rotation. These come from the DJI metadata (to be specified from observed data; no
  third-party code) or from defaults with a manual calibration UI.
- **DJI metadata track.** Parse it clean-room from observed samples: per-frame gyro and
  accelerometer data for stabilisation and horizon levelling, and lens calibration. Treat every
  field as hostile (AGENTS.md §0.4).
- **UI and agent surface.**
  - Show "dual-fisheye 360" in the project panel and clip properties
    (`SphericalLayout::label`).
  - Expose `spherical` through the control channel and MCP media info.
  - Add a 360° viewer (pan/tilt) in the monitors.
  - Add sequence settings for 360°/VR output.
- **Export.** Equirectangular export with spherical metadata (Spherical Video V2 `sv3d`/`st3d` boxes
  in MP4), from the published spec.
- **Proxies.** Proxy jobs for `.osv` should transcode both lenses (or the stitched picture), not only
  lens 0.
