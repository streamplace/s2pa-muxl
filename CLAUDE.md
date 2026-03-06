# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**muxl** is a Rust tool for deterministic MP4 file canonicalization, part of the MUXL specification. It's like [DRISL](https://dasl.ing/drisl.html) but for MP4 files. The goal is to produce byte-identical MP4 output from the same logical content, enabling stable content-addressed identifiers (CIDs) for video.

This is a companion to **S2PA** (Simple Standard for Provenance and Authenticity), which extends C2PA with decentralized identity (DIDs, secp256k1/ES256K signing) for video provenance without certificate authorities. Together, S2PA and MUXL bring video to DASL and the AT Protocol ecosystem.

## Build Commands

```bash
cargo build          # build
cargo run -- <file>  # run against an MP4 file (e.g. cargo run -- samples/file.mp4)
cargo check          # type-check without building
```

## Repo Structure

- `src/main.rs` — Rust binary (reading + canonicalization)
- `spec/` — canonical form specification (one section per box type)
- `scripts/remux.sh` — remuxes an MP4 through ffmpeg, ffmpeg+faststart, gstreamer, MP4Box
- `scripts/mp4dump.py` — machine-readable MP4 box tree dump (supports `--flat` for diffing)
- `web/compare.html` — visual side-by-side comparison of mp4dump output
- `samples/` — test MP4 fixtures
- `Dockerfile` — builds a container with all four muxers for comparison

## Architecture

Currently a single-file binary (`src/main.rs`) that reads an MP4 file and prints metadata (ftyp, moov, tracks, duration, etc.). Uses a local fork of `mp4-rust` at `../mp4-rust` (path dependency in Cargo.toml).

The project is early-stage. The reference implementation targets Rust/WASM and will eventually support:
- **Canonicalization**: arbitrary MP4 → MUXL canonical form (deterministic atom ordering, timestamp bases, chunk layout)
- **Concatenation**: combining MUXL segments while preserving per-segment signatures
- **Segmentation**: splitting MUXL files back into segments

## Key Details

- Rust edition 2024
- Depends on a local `mp4` crate at `../mp4-rust` — this must be present to build
- `samples/file.mp4` is a test fixture

## Comparison Tooling

Generate remuxed variants and compare their box-level structure:

```bash
# Build the comparison container (has ffmpeg, gstreamer, MP4Box)
docker build -t muxl-compare .

# Remux a file through all four muxers → output/ directory
docker run --rm -v $(pwd):/work muxl-compare /work/scripts/remux.sh /work/samples/file.mp4

# Dump flat box structure for diffing
python3 scripts/mp4dump.py --flat samples/output/ffmpeg-faststart.mp4

# Diff two muxer outputs
diff <(python3 scripts/mp4dump.py --flat output/ffmpeg-faststart.mp4) \
     <(python3 scripts/mp4dump.py --flat output/gstreamer.mp4)

# Visual comparison: open web/compare.html in a browser
```

## Canonicalization Workflow

Development follows an incremental, box-by-box process:

1. **Observe discrepancies** — use `mp4dump.py --flat` diffs across muxer outputs to see how a specific box type varies
2. **Document the canonical choice** — add/update the relevant section in `spec/canonical-form.md` with the chosen canonical form and rationale
3. **Implement in Rust** — add the canonicalization logic in `canonicalize()` (or equivalent), with a comment referencing the spec section
4. **Verify** — confirm the output matches the canonical form for test fixtures
5. **Commit** — commit spec + implementation together, one box at a time

All choices are provisional — expect to revisit after real-world playback testing across browsers, mobile players, and hardware decoders.
