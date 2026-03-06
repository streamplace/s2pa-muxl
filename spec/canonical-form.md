# MUXL Canonical Form Specification

This document defines the canonical MP4 box structure produced by MUXL. Each section specifies the canonical choice for a box type, with rationale drawn from observed muxer discrepancies.

All choices are provisional and subject to revision after playback testing.

## Top-Level Box Ordering

<!-- Canonical ordering of top-level boxes (ftyp, moov, mdat, etc.) -->

## ftyp (File Type Box)

<!-- Major brand, minor version, compatible brands list -->

## moov (Movie Box)

### Box Ordering Within moov

<!-- Order of mvhd, trak, udta, etc. within moov -->

### mvhd (Movie Header Box)

<!-- Timescale, duration, version, creation/modification times, matrix, etc. -->

### trak (Track Box)

#### tkhd (Track Header Box)

<!-- Track ID, flags, dimensions, creation/modification times, matrix -->

#### edts (Edit Box)

<!-- Edit list handling -->

#### mdia (Media Box)

##### mdhd (Media Header Box)

<!-- Timescale, duration, language -->

##### hdlr (Handler Box)

<!-- Handler type, name string -->

##### minf (Media Information Box)

###### vmhd / smhd (Video/Sound Media Header)

<!-- Media header flags and defaults -->

###### dinf (Data Information Box)

<!-- Data reference handling -->

###### stbl (Sample Table Box)

<!-- This is where most muxer variation occurs -->

- **stsd** (Sample Description) — codec configuration boxes
- **stts** (Decoding Time to Sample)
- **stss** (Sync Sample) — keyframe table
- **ctts** (Composition Time to Sample) — PTS vs DTS offsets
- **stsc** (Sample to Chunk)
- **stsz** (Sample Size)
- **stco / co64** (Chunk Offset) — 32-bit vs 64-bit decision

## mdat (Media Data Box)

<!-- Sample ordering within mdat, alignment, interleaving strategy -->

## udta (User Data Box)

<!-- Metadata handling — strip, preserve, or normalize -->

## free / skip (Free Space Boxes)

<!-- Policy on padding boxes -->
