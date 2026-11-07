# DJI Osmo 360 `.OSV` import

Status: **phase 2 done**: `.osv` files import as clips marked *dual-fisheye 360*, and every frame
holds both lenses side by side (see [Phase 2](#phase-2-both-lenses-in-one-frame)). Nothing is
stitched yet: the monitors show the two fisheye circles next to each other.

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
5. **What plays** (phase 1; phase 2 changed it, see below). `MediaInfo::video` described lens 0
   (the first lens track in file order) and only lens 0 was decoded. `Mp4Source::fisheye_lens_tracks()` returns both track indices for phase 2.
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
- **Decode both lenses.** Done in phase 2 (below).
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

## Phase 2: both lenses in one frame

### Frame layout

A dual-fisheye clip delivers **one frame per time**, `2W`×`H` for lenses of `W`×`H`:

```text
+-------------------+-------------------+
|  lens 0           |  lens 1           |
|  track v:0        |  track v:1        |
|  (file order)     |  (file order)     |
+-------------------+-------------------+
x = 0 ..W           x = W .. 2W
```

- Lens 0 is the first lens track in file order (`fisheye_lens_tracks()[0]`, ffmpeg's `0:v:0`), lens 1
  the second. The layout is exactly `ffmpeg -filter_complex "[0:v:0][0:v:1]hstack"`.
- `MediaInfo::video.width` is `2W` (height, rate, codec and pixel format are the lenses'), so
  sequences, proxies, thumbnails and exports see the real frame size. A width whose double would
  overflow `u32` plays lens 0 alone as flat video.
- Bit depth is kept. Main 10 lenses give a 10-bit `Yuv16` frame and are never reduced to 8 bit. If
  one lens comes out 8-bit and the other deeper (a hardware and a software decoder), the 8-bit side
  is widened (`v << 2` for 8 → 10), never the reverse (`VideoFrame::hstack`, `crates/frame`).
- Colour, pixel aspect and pts are lens 0's. Lens 1 uses lens 0's clean aperture and display
  rotation (one camera, one geometry), so the halves always have the same size.

**Which lens is which (front/back, the side with the screen).** I can't tell from the data I have.
The stand-ins are synthetic, and no camera file was available for this phase. The DJI metadata
track (not parsed yet) or a test shot with a known front scene will settle it. Until then the
code says only "lens 0 / lens 1 = track order". Nothing depends on which is front.

### Decoding

- `Mp4Source` has a second `GopCache` (`Lens1`) for lens 1, with its own decoder. On a frame request
  lens 1 is decoded on a scoped thread of its own (`decode-lens-1`, carrying the request's
  cancel/draft/background hints; a panic there becomes the frame's error), while lens 0 is decoded on
  the calling thread. Then the two frames are put side by side.
- **Pairing by PTS.** Both lens tracks get the same timing normalisation: `snap_to_frame_grid`, and
  the same `rebase_unedited_video` shift in ticks (`shift_like_lens_0`). Each lens then looks up its
  frame for the same `Tick` through its own timescale. Frames the camera took together therefore pair
  even when the tracks' units differ. Edit lists are applied per track as for any MP4.
- **Seek.** Each lens's cache seeks to its own sync sample at or before the target (open-GOP leading
  pictures included) and decodes on to the target PTS. When the lenses share a GOP raster, as
  expected from one encoder, that is the same keyframe for both. When they don't, each lens still
  lands on the exact frame. The Main 10 stand-in has deliberately different GOPs per lens (keyint
  12 vs 9) to cover that case.
- **Repeat requests** (a redraw, a paused playhead) get the last side-by-side frame back without
  copying when both lens caches return the same frames. A replaced side-by-side frame goes back to the
  plane pool.
- **Hardware decoding.** Real platform factories already decline a stream whose session can't be
  created, and the next factory or the software decoder takes it. A factory that returns an *error*
  instead (for example, no second session) would have failed the lens. Lens tracks therefore fall
  back to `software_video_decoder` when `make_video_decoder` fails. A hardware decoder that fails
  mid-stream already switches to software (`HybridDecoder`). Normal MP4 tracks keep the old
  behaviour.
- **Normal MP4 is unchanged.** It has no `Lens1`, uses the same lookup through `track_frame`, takes
  no extra copy, and starts no thread.

### Tests

- `crates/codecs/tests/it/osv_dual_fisheye.rs` (ffmpeg stand-ins: libx265, two lenses with
  different content, B-frames, 2 s at 25 fps):
  - 8-bit and Main 10: every frame, played from the start, matches
    `ffmpeg -filter_complex "[0:v:0][0:v:1]hstack"` **bit-exactly**. These are the same criteria as
    the other HEVC oracle tests (`mp4_oracle.rs`: plane equality). The Main 10 frames stay 10-bit.
  - Seek: the frame at t = 1.5 s (frame 37) equals linear playback after a cold seek, a seek back
    from the last frame, and a seek from another GOP (8-bit and Main 10).
  - The same file named `.mp4` plays `v:0` alone, `W`×`H`, exactly as ffmpeg decodes it.
  - Hardware fallback: a stand-in factory hands out one "hardware" session and fails every further
    one. The clip still matches ffmpeg's hstack.
  - Camera file (`$FILMCRAFT_OSV_SAMPLE` or `target/fixtures/osv/sample.osv`): the same oracle and
    seek check. It is skipped without the file.
  - Sanity checks while writing: with the software fallback disabled the hardware test fails, and
    with the lenses swapped the oracle tests fail.
- `crates/codecs/src/mp4/osv_tests.rs`: the reported size is `2W`. The truncation/bit-flip fuzz now
  also asks both lenses for a frame.
- `crates/frame`: `hstack` unit tests (row layout for 4:2:0/4:2:2/4:4:4, 10 bit kept, 8 → 10
  widening, refusals).
- `osv_import.rs` (phase 1): updated to `2W` frames.

### Performance

Measured with `cargo test -p filmcraft-codecs --test it dual_fisheye_playback_fps -- --ignored
--nocapture` (test profile, which is optimised). The machine was a 2-thread cloud VM, so these are
relative numbers, not a target machine.

**No `sample.osv` was available**, so the numbers are for a stand-in: 2 × 1920×1920 HEVC Main 10,
4:2:0, 25 fps, 50 frames, played from the start.

| What | fps |
|---|---|
| both lenses, side by side (3840×1920) | **≈ 22** |
| lens 0 alone (1920×1920) | ≈ 80–87 |
| lens 1 alone (1920×1920, busier test pattern) | ≈ 45 |

- **Parallel vs serial.** Decoding the lenses one after the other measured the same (≈ 21–23 fps)
  on 2 threads. The HEVC decoder already keeps both cores busy, so the second decoding thread only
  pays off with more cores.
- **The side-by-side copy** costs ≈ 8–10 ms per 3840×1920 10-bit frame, about 20 % of the frame
  time here. A first draft copied every plane twice when both lenses had the same bit depth. Now
  each plane is copied once, rows in parallel, into pooled buffers.
- **Content matters.** A noisy lens 1 alone took 4.2 s for 50 frames versus 0.6 s for a clean lens 0.
  Measure on the camera file before drawing conclusions:
  `FILMCRAFT_OSV_SAMPLE=/path/sample.osv cargo test -p filmcraft-codecs --test it dual_fisheye_playback_fps -- --ignored --nocapture`.

### Open points after phase 2

- **Camera file.** Run the oracle, seek and fps tests on the real `sample.osv`, then record:
  - lens size, bit depth (Main or Main 10), frame rate and GOP raster per lens;
  - whether the two tracks' edit lists and first PTS agree;
  - the fps at the camera's resolution.

  Get the file into a pinned corpus for CI (phase 1 open point).
- **Front/back lens** (see above): settle with the metadata track or a test shot.
- **Copy cost.** The `2W` frame is copied on the CPU. For 2 × 4K Main 10, a render path that takes the
  two lens frames separately (uploaded as two textures, stitched on the GPU) would avoid the copy.
  That fits the projection step below.
- **Cache budget.** Two caches of a large HEVC stream share the global frame and decoder budget
  (`MAX_LIVE_DECODERS`, `FRAME_BUDGET`). Playback of several OSV clips may evict sooner. Look at it
  with real files.
- **A damaged lens 1** fails the whole frame (as a damaged frame of any clip does). Showing lens 0 with
  a black right half instead is a possible later refinement.
- **Hardware sessions.** With a real GPU, check the counters (`perf.stats`: sessions, declined,
  fallbacks) to see whether both lenses get a hardware session or one runs in software.
- Unchanged from phase 1: projection, metadata track, UI/MCP surface, export, proxies.
