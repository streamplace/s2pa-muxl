use std::io::{self, Read, Seek, SeekFrom, Write};

use mp4::{BoxHeader, BoxType, FtypBox, Mp4Reader, MoovBox, WriteBox};

/// Errors returned by muxl operations.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Mp4(mp4::Error),
    InvalidMp4(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Mp4(e) => write!(f, "MP4 error: {e}"),
            Error::InvalidMp4(msg) => write!(f, "invalid MP4: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<mp4::Error> for Error {
    fn from(e: mp4::Error) -> Self {
        Error::Mp4(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// Canonical ftyp: isom, minor version 0, compatible brands [isom, iso2, avc1, mp41].
// Spec: canonical-form.md § ftyp
fn canonical_ftyp() -> FtypBox {
    FtypBox {
        major_brand: str::parse("isom").unwrap(),
        minor_version: 0,
        compatible_brands: vec![
            str::parse("isom").unwrap(),
            str::parse("iso2").unwrap(),
            str::parse("avc1").unwrap(),
            str::parse("mp41").unwrap(),
        ],
    }
}

// Canonicalize moov in-place: zero timestamps, canonical hdlr names, strip udta/meta.
// Spec: canonical-form.md § moov, mvhd, tkhd, mdhd, hdlr
fn canonicalize_moov(moov: &mut MoovBox) {
    // mvhd: zero timestamps, version 0, timescale 1000, flags 0
    moov.mvhd.version = 0;
    moov.mvhd.flags = 0;
    moov.mvhd.creation_time = 0;
    moov.mvhd.modification_time = 0;
    // Preserve timescale and duration — they're derived from content.
    // But normalize timescale to 1000 and recompute duration.
    let old_timescale = moov.mvhd.timescale as u64;
    let new_timescale = 1000u64;
    if old_timescale != new_timescale && old_timescale != 0 {
        moov.mvhd.duration = moov.mvhd.duration * new_timescale / old_timescale;
    }
    moov.mvhd.timescale = new_timescale as u32;

    // Sort tracks by track_id for deterministic order.
    moov.traks.sort_by_key(|t| t.tkhd.track_id);

    for trak in &mut moov.traks {
        // tkhd: zero timestamps, flags = 3 (enabled + in_movie), version 0
        trak.tkhd.version = 0;
        trak.tkhd.flags = 3; // track_enabled | track_in_movie
        trak.tkhd.creation_time = 0;
        trak.tkhd.modification_time = 0;
        // Recompute tkhd.duration in new movie timescale
        let media_timescale = trak.mdia.mdhd.timescale as u64;
        let media_duration = trak.mdia.mdhd.duration;
        if media_timescale != 0 {
            trak.tkhd.duration = media_duration * new_timescale / media_timescale;
        }

        // mdhd: zero timestamps, version 0, preserve timescale/duration/language
        trak.mdia.mdhd.version = 0;
        trak.mdia.mdhd.flags = 0;
        trak.mdia.mdhd.creation_time = 0;
        trak.mdia.mdhd.modification_time = 0;

        // hdlr: canonical handler names
        trak.mdia.hdlr.version = 0;
        trak.mdia.hdlr.flags = 0;
        let handler_type: String = trak.mdia.hdlr.handler_type.to_string();
        trak.mdia.hdlr.name = match handler_type.as_str() {
            "vide" => "VideoHandler".to_string(),
            "soun" => "SoundHandler".to_string(),
            "sbtl" | "text" => "SubtitleHandler".to_string(),
            _ => String::new(),
        };

        // Strip trak-level meta
        trak.meta = None;
    }

    // Recompute next_track_id
    moov.mvhd.next_track_id = moov
        .traks
        .iter()
        .map(|t| t.tkhd.track_id)
        .max()
        .unwrap_or(0)
        + 1;

    // Recompute mvhd.duration as max of track durations (in movie timescale)
    moov.mvhd.duration = moov.traks.iter().map(|t| t.tkhd.duration).max().unwrap_or(0);

    // Strip udta and moov-level meta (tool tags, etc.)
    moov.udta = None;
    moov.meta = None;
}

/// Transform an arbitrary MP4 into MUXL canonical form.
///
/// Reads a complete MP4 from `input` and writes the canonicalized MP4 to `output`.
/// The output is byte-deterministic: the same logical content always produces
/// identical bytes.
pub fn canonicalize<RS: Read + Seek, WS: Write + Seek>(mut input: RS, mut output: WS) -> Result<()> {
    let end = input.seek(SeekFrom::End(0))?;
    input.seek(SeekFrom::Start(0))?;
    let mut reader = Mp4Reader::read_header(input, end)?;
    canonicalize_from_reader(&mut reader, &mut output)
}

fn canonicalize_from_reader<RS: Read + Seek, WS: Write + Seek>(
    reader: &mut Mp4Reader<RS>,
    writer: &mut WS,
) -> Result<()> {
    // 1. Write canonical ftyp
    let ftyp = canonical_ftyp();
    ftyp.write_box(writer)?;

    // 2. Write mdat with placeholder size, then copy all samples per-track
    let mdat_pos = writer.stream_position()?;
    // Placeholder mdat header (will be fixed later)
    BoxHeader::new(BoxType::MdatBox, 0).write(writer)?;

    let mut moov = reader.moov.clone();

    // Collect track IDs sorted
    let mut track_ids: Vec<u32> = reader.tracks().keys().copied().collect();
    track_ids.sort();

    // For each track, write all samples sequentially. Record chunk offsets.
    // Canonical layout: one sample per chunk, tracks written sequentially.
    for &track_id in &track_ids {
        let sample_count = reader.sample_count(track_id)?;

        let mut chunk_offsets: Vec<u64> = Vec::with_capacity(sample_count as usize);

        for sample_id in 1..=sample_count {
            let offset = writer.stream_position()?;
            chunk_offsets.push(offset);

            let sample = reader
                .read_sample(track_id, sample_id)?
                .ok_or_else(|| Error::InvalidMp4(format!("missing sample {sample_id} in track {track_id}")))?;
            writer.write_all(&sample.bytes)?;
        }

        // Update stbl for this track: one sample per chunk
        let trak = moov
            .traks
            .iter_mut()
            .find(|t| t.tkhd.track_id == track_id)
            .unwrap();

        // stsc: single entry — every chunk has 1 sample
        trak.mdia.minf.stbl.stsc.entries.clear();
        trak.mdia.minf.stbl.stsc.entries.push(mp4::StscEntry {
            first_chunk: 1,
            samples_per_chunk: 1,
            sample_description_index: 1,
            first_sample: 1,
        });

        // Use stco (32-bit) if all offsets fit, otherwise co64
        let max_offset = chunk_offsets.iter().copied().max().unwrap_or(0);
        if max_offset <= u32::MAX as u64 {
            trak.mdia.minf.stbl.stco = Some(mp4::StcoBox::default());
            trak.mdia.minf.stbl.stco.as_mut().unwrap().entries =
                chunk_offsets.iter().map(|&o| o as u32).collect();
            trak.mdia.minf.stbl.co64 = None;
        } else {
            trak.mdia.minf.stbl.co64 = Some(mp4::Co64Box::default());
            trak.mdia.minf.stbl.co64.as_mut().unwrap().entries = chunk_offsets;
            trak.mdia.minf.stbl.stco = None;
        }
    }

    // 3. Fix mdat size
    let mdat_end = writer.stream_position()?;
    let mdat_size = mdat_end - mdat_pos;
    writer.seek(SeekFrom::Start(mdat_pos))?;
    BoxHeader::new(BoxType::MdatBox, mdat_size).write(writer)?;
    writer.seek(SeekFrom::Start(mdat_end))?;

    // 4. Canonicalize moov metadata
    canonicalize_moov(&mut moov);

    // 5. Write moov
    moov.write_box(writer)?;

    Ok(())
}

/// Split a MUXL canonical MP4 into independently-signable segments.
///
/// Reads a canonical MP4 from `input` and writes segments to `output`.
/// The segment format is TBD.
pub fn segment<RS: Read + Seek, WS: Write + Seek>(_input: RS, _output: WS) -> Result<()> {
    todo!("segment: not yet implemented")
}

/// Concatenate MUXL segments into a single canonical MP4.
///
/// Reads segments from `inputs` and writes the combined MP4 to `output`.
/// Per-segment signatures are preserved.
pub fn concatenate<RS: Read + Seek, WS: Write + Seek>(
    _inputs: &mut [RS],
    _output: WS,
) -> Result<()> {
    todo!("concatenate: not yet implemented")
}
