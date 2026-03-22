# MUXL Canonical Form Specification

This document defines the canonical byte layout for MUXL segments and derived formats.

All choices are provisional and subject to revision after playback testing.

## MUXL Segment

A MUXL segment contains one track's data for one GoP. Each frame gets its own moof+mdat pair. Segments are per-track — a GoP with N tracks produces N segments.

```
Segment (track 1, GoP 1):
  moof(frame 1) + mdat(frame 1)
  moof(frame 2) + mdat(frame 2)
  ...

Segment (track 2, GoP 1):
  moof(frame 1) + mdat(frame 1)
  moof(frame 2) + mdat(frame 2)
  ...
```

Per-track segments enable byte-range addressing (HLS playlists can index a single archive file) and independent per-track content hashing. Segments for the same track are blindly concatenatable by byte appending.

Track initialization metadata (codec config, timescales) is out-of-band — either in the archive file's init segment or from an external source.

### Segmentation Rule

Segment boundaries are driven by video sync samples (keyframes). Audio samples are grouped with the video GoP they temporally overlap. Given the same samples with the same timestamps, the segment boundaries are always identical.

### moof

Each moof covers exactly one sample (frame) from one track.

- **mfhd**: sequence_number, 1-based, incrementing globally across the stream
- **traf**: exactly one per moof
  - **tfhd**: track_id; flags = `default_base_is_moof`; no default sample values (all explicit in trun)
  - **tfdt**: base_media_decode_time in the track's media timescale
  - **trun**: exactly one entry; flags = `data_offset | sample_duration | sample_size | sample_flags`; add `sample_cts` flag if the sample has a non-zero composition time offset

### trun Sample Flags

- Sync sample: `0x02000000` (sample_depends_on = 2: depends on no other sample)
- Non-sync sample: `0x01010000` (sample_depends_on = 1: depends on others; sample_is_non_sync = 1)

### mdat

One mdat per moof, containing exactly one sample's data.

## MUXL Archive fMP4

Init segment followed by per-track segments grouped by track.

```
ftyp
moov (init — track config, empty sample tables)
[track 1 segments: GoP 1, GoP 2, ...]
[track 2 segments: GoP 1, GoP 2, ...]
...
```

Valid fMP4 file. Each track's segments form a contiguous byte range, enabling HLS byte-range playlists to address individual tracks within a single file. Tracks are ordered by track_id ascending.

## ftyp

- **major_brand**: `isom`
- **minor_version**: `0`
- **compatible_brands**: `[isom, iso2]`

Codec-agnostic. Players use stsd entries for codec detection.

## Init Segment moov

The moov in the init segment describes track configuration with empty sample tables, zero durations, and no sample entries.

Required child boxes: `mvhd`, `trak` (one per track), `mvex` (with `trex` per track).

### mvhd

- **version**: 0
- **flags**: 0
- **creation_time**: 0
- **modification_time**: 0
- **timescale**: 1000
- **duration**: 0
- **rate**: 1.0
- **volume**: 1.0
- **matrix**: identity
- **next_track_id**: max(track_ids) + 1

### mvex

Required for fMP4 playback — signals that moof+mdat pairs follow the moov.

- **trex** (one per track):
  - **track_id**: matching the trak
  - **default_sample_description_index**: 1
  - **default_sample_duration**: 0
  - **default_sample_size**: 0
  - **default_sample_flags**: 0

All sample metadata is explicit in each trun entry, so trex defaults are all zero.

### trak ordering

Sorted by track_id ascending. No udta, meta, or iods.

### tkhd

- **version**: 0
- **flags**: 3 (track_enabled | track_in_movie)
- **creation_time**: 0
- **modification_time**: 0
- **duration**: 0
- **matrix, width/height, layer, alternate_group, volume**: from track config

### mdhd

- **version**: 0
- **flags**: 0
- **creation_time**: 0
- **modification_time**: 0
- **timescale**: preserved from source track (passthrough)
- **duration**: 0
- **language**: `"und"`

### hdlr

- **version**: 0
- **flags**: 0
- **handler_type**: `"vide"` for video, `"soun"` for audio
- **name**: empty string (name is cosmetic and varies across muxers)

### minf

- **vmhd**: present for video tracks (default values)
- **smhd**: present for audio tracks (default values)
- **dinf**: required, contains dref
  - **dref**: one self-contained `url` entry with empty location string (signals data is in the same file)

### stbl (Sample Table)

stsd populated with codec config, all other tables empty.

### edts / elst

From track config. segment_duration in movie timescale (1000). media_time in the track's media timescale.

## Stripped Boxes

The following are stripped entirely:

- **udta**: tool tags are non-deterministic
- **meta**: at moov and trak level
- **free / skip**: padding boxes
- **iods**: not needed
