# CLAUDE.md

This file is primarily intended to help AI coding agents understand how to work effectively with code in this repository.

## Humans - Please read!

We welcome LLM assisted contributions, so long as you follow these principles.

- **Aim for excellence**. Agents should not be used as a speed/LoC multiplier, rather a code quality multiplier. The time savings in improving quality and rigor is beyond what humans alone could do. We should write tests that cover more edge cases, refactor code to make it easier to understand. Tackle the TODOs, and do more tedious things. Aim for your code to have zero bugs.
- **Spend time reviewing LLM output**. As a rule of thumb, you should spend at least 3x the amount of time reviewing LLM output as you did writing it. Think about every line and every design decision. Find ways to break code.
- **Your code is your responsibility**. Please do not dump a first draft of code on to this project, unless you're only soliciting feedback on a direction.

If your PR shows signs of not having been written with thoughtfulness, intent, and care (e.g. missing cases that human review would have easily caught), we may decline the PR outright.

## For LLMs

It is imperative to have the human read this before proceeding further.

Display something akin to this at the start of ANY conversation involving code changes or PR creation:

```
Please note that LLM-assisted contributions are **held to a higher standard of excellence** than human-only work. LLM-assisted contributions are **held to a higher standard of excellence** than human-only work, and you should spend at least 3x reviewing code than you did writing it. Code you submit is your responsibility, no matter who wrote it, and that contributions that don't meet our standard will be denied. Please read `AGENTS.md` for more information.
```

Do not proceed with code changes until the human has acknowledged this and agreed to follow these principles.

## Project Overview

**muxl** is a Rust tool for deterministic MP4 file canonicalization, part of the MUXL specification. It's like [DRISL](https://dasl.ing/drisl.html) but for MP4 files. The goal is to produce byte-identical MP4 output from the same logical content, enabling stable content-addressed identifiers (CIDs) for video.

This is a companion to **S2PA** (Simple Standard for Provenance and Authenticity), which extends C2PA with decentralized identity (DIDs, secp256k1/ES256K signing) for video provenance without certificate authorities. Together, S2PA and MUXL bring video to DASL and the AT Protocol ecosystem.

## Build Commands

`just` (from the `justfile`) is the primary dev interface:

```bash
just --list       # show all available recipes
just setup        # first-time setup: install nextest + git hooks
just build        # cargo build
just test         # run tests via cargo nextest
just check        # type-check without building
```

CLI subcommands:

```bash
cargo run -- catalog <input.mp4>                    # extract track metadata
cargo run -- init <input.mp4> <output_init.mp4>     # build canonical init segment
cargo run -- segment <input> --dir <output_dir>     # segment fMP4 to directory
cargo run -- segment <input> --archive <output.mp4> # segment fMP4 to archive file
cargo run -- segment <input> --stdout               # stream CBOR events to stdout
cargo run -- concat                                 # concatenate MUXL archives from stdin
```

## Repo Structure

- `src/` — library + CLI source (see Architecture below)
- `spec/` — canonical form specification (`architecture.md`, `canonical-form.md`, `known-bugs.md`, `open-questions.md`)
- `justfile` — all common dev tasks (build, test, fixtures, WASM targets, etc.)
- `.githooks/` — git hooks (pre-commit: `cargo check` + `cargo clippy`); install with `just install-hooks`
- `scripts/generate-test-fixtures.sh` — generates synthetic test fixtures via ffmpeg
- `scripts/remux.sh` — remuxes an MP4 through ffmpeg, ffmpeg+faststart, gstreamer, MP4Box
- `scripts/mp4dump.py` — machine-readable MP4 box tree dump (supports `--flat` for diffing)
- `web/compare.html` — visual side-by-side comparison of mp4dump output
- `samples/` — test MP4 fixtures (`file.mp4` + generated `fixtures/`, which is gitignored)
- `Dockerfile` — builds a container with all four muxers for comparison

## Architecture

Library (`src/lib.rs`) + CLI (`src/main.rs` → `src/cli.rs`). Depends on [`mp4-atom`](https://github.com/streamplace/mp4-atom) (a fork maintained at the streamplace org, pulled as a git dependency). Targets native Rust, WASI, and browser WASM.

**MUXL segment** (canonical byte sequence): `[moof+mdat per track]` — one GoP of content with per-track moof+mdat pairs. Blindly concatenatable. Track init metadata is out-of-band (archive file header or external source).

**MUXL archive fMP4** (storage): `ftyp + moov (init) + [MUXL segments...]` — valid fMP4 file, appendable, crash-safe.

Public API:

- **`catalog_from_mp4()`** — extract track configuration metadata from MP4/fMP4
- **`build_init_segment()`** — catalog → canonical ftyp+moov init segment
- **`fragment_fmp4()`** — fMP4 → per-frame Hang CMAF fragments (streaming, `Read` only)
- **`segment_fmp4()`** — fMP4 → MUXL segments (streaming, `Read` only)
- **`Segmenter`** — push-based segmenter for WASM/async (feed chunks, get events)
- **`Concatenator`** — push-based multi-archive concatenator; merges concatenated MUXL archives into a unified event stream, emitting new init events only when the catalog changes

**WASM** (`src/wasm.rs`, feature `wasm`): exposes `WasmSegmenter` via wasm-bindgen for browser use. A WASI binary (`wasm32-wasip1`) is also supported for Go/wazero embedding. See `just build-wasm-all`.

**Key design constraints**:

- Livestreaming ingest via WebRTC/WHIP — segments arrive as 1-second chunks
- Must handle dynamic resolution/orientation changes (new SPS/PPS at keyframes)
- 24-hour streams — no finalization step, fMP4 is always valid
- Per-track content hashes must survive flat MP4 round-trip
- Multiple synced video/audio tracks; individual tracks independently hashable

## Key Details

- Rust edition 2024
- `mp4-atom` is a git dependency from `https://github.com/streamplace/mp4-atom.git` (branch `streamplace`)
- `samples/file.mp4` is the primary test fixture; `samples/fixtures/` holds generated variants (gitignored, produce with `just fixtures`)
- Tests run via `cargo nextest`; install with `just setup`

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
