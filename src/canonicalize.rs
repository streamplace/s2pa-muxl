//! Canonicalization: arbitrary MP4 → MUXL canonical flat MP4.
//! Spec: canonical-form.md

use std::io::{Read, Seek, SeekFrom, Write};

use mp4::{BoxHeader, BoxType, FtypBox, Mp4Reader, MoovBox, WriteBox};

use crate::error::{Error, Result};
use crate::sample_table::{build_canonical_stsc, resolve_sample_description_indices};
use crate::timescale::{canonical_timescale_for_handler, rescale_track_timescale};

// Canonical ftyp: isom, minor version 0, compatible brands [isom, iso2].
// Spec: canonical-form.md § ftyp
pub(crate) fn canonical_ftyp() -> FtypBox {
    FtypBox {
        major_brand: str::parse("isom").unwrap(),
        minor_version: 0,
        compatible_brands: vec![
            str::parse("isom").unwrap(),
            str::parse("iso2").unwrap(),
        ],
    }
}

// Canonicalize moov in-place: zero timestamps, canonical hdlr names, strip udta/meta.
// Spec: canonical-form.md § moov, mvhd, tkhd, mdhd, hdlr
fn canonicalize_moov(moov: &mut MoovBox) -> Result<()> {
    // mvhd: zero timestamps, version 0, timescale 1000, flags 0
    moov.mvhd.version = 0;
    moov.mvhd.flags = 0;
    moov.mvhd.creation_time = 0;
    moov.mvhd.modification_time = 0;
    let old_movie_timescale = moov.mvhd.timescale as u64;
    let new_timescale = 1000u64;
    if old_movie_timescale != new_timescale && old_movie_timescale != 0 {
        moov.mvhd.duration = moov.mvhd.duration * new_timescale / old_movie_timescale;
    }
    moov.mvhd.timescale = new_timescale as u32;

    // Sort tracks by track_id for deterministic order.
    moov.traks.sort_by_key(|t| t.tkhd.track_id);

    for trak in &mut moov.traks {
        // tkhd: zero timestamps, flags = 3 (enabled + in_movie), version 0
        trak.tkhd.version = 0;
        trak.tkhd.flags = 3;
        trak.tkhd.creation_time = 0;
        trak.tkhd.modification_time = 0;

        // Rescale elst segment_duration from old movie timescale to new
        if let Some(ref mut edts) = trak.edts {
            if let Some(ref mut elst) = edts.elst {
                for entry in &mut elst.entries {
                    if old_movie_timescale != 0 {
                        entry.segment_duration =
                            entry.segment_duration * new_timescale / old_movie_timescale;
                    }
                }
            }
        }

        // Recompute tkhd.duration in new movie timescale
        let media_timescale = trak.mdia.mdhd.timescale as u64;
        let media_duration = trak.mdia.mdhd.duration;
        if media_timescale != 0 {
            trak.tkhd.duration = media_duration * new_timescale / media_timescale;
        }

        // mdhd: zero timestamps, version 0
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

        // Normalize media timescale
        // Spec: canonical-form.md § mdhd
        if let Some(ts) = canonical_timescale_for_handler(&handler_type) {
            rescale_track_timescale(trak, ts)?;
        }

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

    // Recompute mvhd.duration as max of track durations
    moov.mvhd.duration = moov
        .traks
        .iter()
        .map(|t| t.tkhd.duration)
        .max()
        .unwrap_or(0);

    moov.udta = None;
    moov.meta = None;

    Ok(())
}

/// Transform an arbitrary MP4 into MUXL canonical flat MP4 form.
///
/// Reads a complete MP4 from `input` and writes the canonicalized MP4 to `output`.
/// The output is byte-deterministic: the same logical content always produces
/// identical bytes.
pub fn canonicalize<RS: Read + Seek, WS: Write + Seek>(
    mut input: RS,
    mut output: WS,
) -> Result<()> {
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
    BoxHeader::new(BoxType::MdatBox, 0).write(writer)?;

    let mut moov = reader.moov.clone();

    let mut track_ids: Vec<u32> = reader.tracks().keys().copied().collect();
    track_ids.sort();

    // For each track, write all samples sequentially. Record chunk offsets.
    for &track_id in &track_ids {
        let sample_count = reader.sample_count(track_id)?;
        let mut chunk_offsets: Vec<u64> = Vec::with_capacity(sample_count as usize);

        for sample_id in 1..=sample_count {
            let offset = writer.stream_position()?;
            chunk_offsets.push(offset);

            let sample = reader
                .read_sample(track_id, sample_id)?
                .ok_or_else(|| {
                    Error::InvalidMp4(format!(
                        "missing sample {sample_id} in track {track_id}"
                    ))
                })?;
            writer.write_all(&sample.bytes)?;
        }

        let trak = moov
            .traks
            .iter_mut()
            .find(|t| t.tkhd.track_id == track_id)
            .unwrap();

        // stsc: one sample per chunk, preserving sample_description_index changes.
        // Spec: canonical-form.md § Multiple Sample Descriptions
        let sample_desc_indices = resolve_sample_description_indices(
            &trak.mdia.minf.stbl.stsc.entries,
            sample_count,
        );
        trak.mdia.minf.stbl.stsc.entries = build_canonical_stsc(&sample_desc_indices);

        // stco/co64
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
    canonicalize_moov(&mut moov)?;

    // 5. Write moov
    moov.write_box(writer)?;

    Ok(())
}
