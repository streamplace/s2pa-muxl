# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**muxl** is a Rust tool for deterministic MP4 file canonicalization, part of the MUXL specification. It's like [DRISL](https://dasl.ing/drisl.html) but for MP4 files. The goal is to produce byte-identical MP4 output from the same logical content, enabling stable content-addressed identifiers (CIDs) for video.

This is a companion to **S2PA** (Simple Standard for Provenance and Authenticity), which extends C2PA with decentralized identity (DIDs, secp256k1/ES256K signing) for video provenance without certificate authorities. Together, S2PA and MUXL bring video to DASL and the AT Protocol ecosystem.

## Build Commands

```bash
cargo build          # build
cargo run -- <file>  # run against an MP4 file (e.g. cargo run -- file.mp4)
cargo check          # type-check without building
```

## Architecture

Currently a single-file binary (`src/main.rs`) that reads an MP4 file and prints metadata (ftyp, moov, tracks, duration, etc.). Uses a local fork of `mp4-rust` at `../mp4-rust` (path dependency in Cargo.toml).

The project is early-stage. The reference implementation targets Rust/WASM and will eventually support:
- **Canonicalization**: arbitrary MP4 → MUXL canonical form (deterministic atom ordering, timestamp bases, chunk layout)
- **Concatenation**: combining MUXL segments while preserving per-segment signatures
- **Segmentation**: splitting MUXL files back into segments

## Key Details

- Rust edition 2024
- Depends on a local `mp4` crate at `../mp4-rust` — this must be present to build
- The `file.mp4` in the repo root is a test fixture
