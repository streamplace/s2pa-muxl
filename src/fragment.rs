//! Fragmentation: flat MP4 → per-frame fMP4 fragments (Hang CMAF style).
//!
//! Each track is fragmented independently. Each frame becomes a single
//! moof+mdat pair. The fragment function produces concatenated moof+mdat
//! pairs for a single track, suitable for Hang-style per-frame CMAF delivery.
//!
//! Spec: architecture.md § Hang CMAF

use std::io::{Read, Seek, SeekFrom, Write};

use mp4_atom::{
    CttsEntry, Encode, Mfhd, Moof, StscEntry, StszSamples, SttsEntry, Tfdt, Tfhd, Traf, Trun,
    TrunEntry,
};

use crate::error::{Error, Result};
use crate::init::read_moov;

/// Per-sample metadata extracted from the moov sample tables.
struct SampleInfo {
    duration: u32,
    size: u32,
    is_sync: bool,
    cts_offset: i32,
    /// File offset where this sample's data starts.
    file_offset: u64,
}

/// Per-track metadata needed for fragmentation.
struct TrackInfo {
    samples: Vec<SampleInfo>,
}

/// Extract per-sample metadata from a track's sample tables.
///
/// Resolves stsc chunk layout, stco/co64 chunk offsets, stts durations,
/// ctts offsets, stss sync samples, and stsz sizes into a flat per-sample
/// list with file offsets for reading sample data.
fn extract_track_info(trak: &mp4_atom::Trak) -> Result<TrackInfo> {
    let stbl = &trak.mdia.minf.stbl;

    // Sample sizes
    let sizes: Vec<u32> = match &stbl.stsz.samples {
        StszSamples::Different { sizes } => sizes.clone(),
        StszSamples::Identical { count, size } => vec![*size; *count as usize],
    };
    let sample_count = sizes.len();

    // Sample durations from stts (run-length encoded)
    let durations = expand_stts(&stbl.stts.entries, sample_count);

    // Composition time offsets from ctts
    let cts_offsets = match &stbl.ctts {
        Some(ctts) => expand_ctts(&ctts.entries, sample_count),
        None => vec![0i32; sample_count],
    };

    // Sync samples from stss (None = all sync)
    let sync_set: Option<std::collections::HashSet<u32>> = stbl.stss.as_ref().map(|stss| {
        stss.entries.iter().copied().collect()
    });

    // Chunk offsets from stco or co64
    let chunk_offsets: Vec<u64> = if let Some(ref stco) = stbl.stco {
        stco.entries.iter().map(|&o| o as u64).collect()
    } else if let Some(ref co64) = stbl.co64 {
        co64.entries.clone()
    } else {
        return Err(Error::InvalidMp4("no stco or co64 box".into()));
    };

    // Resolve stsc to get per-sample file offsets
    let file_offsets = resolve_sample_offsets(&stbl.stsc.entries, &chunk_offsets, &sizes)?;

    let mut samples = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let is_sync = match &sync_set {
            Some(set) => set.contains(&(i as u32 + 1)),
            None => true, // no stss = all sync
        };
        samples.push(SampleInfo {
            duration: durations[i],
            size: sizes[i],
            is_sync,
            cts_offset: cts_offsets[i],
            file_offset: file_offsets[i],
        });
    }

    Ok(TrackInfo { samples })
}

/// Resolve stsc entries + chunk offsets + sample sizes into per-sample file offsets.
fn resolve_sample_offsets(
    stsc_entries: &[StscEntry],
    chunk_offsets: &[u64],
    sample_sizes: &[u32],
) -> Result<Vec<u64>> {
    let sample_count = sample_sizes.len();
    if sample_count == 0 {
        return Ok(vec![]);
    }
    if stsc_entries.is_empty() {
        return Err(Error::InvalidMp4("empty stsc table".into()));
    }

    let mut offsets = Vec::with_capacity(sample_count);
    let mut sample_idx = 0usize;

    for (i, entry) in stsc_entries.iter().enumerate() {
        let first_chunk = entry.first_chunk as usize;
        let next_first_chunk = if i + 1 < stsc_entries.len() {
            stsc_entries[i + 1].first_chunk as usize
        } else {
            chunk_offsets.len() + 1 // run to end of chunks
        };

        // chunk indices are 1-based
        for chunk_idx in first_chunk..next_first_chunk {
            if chunk_idx < 1 || chunk_idx > chunk_offsets.len() {
                break;
            }
            let mut offset = chunk_offsets[chunk_idx - 1];
            for _ in 0..entry.samples_per_chunk {
                if sample_idx >= sample_count {
                    return Ok(offsets);
                }
                offsets.push(offset);
                offset += sample_sizes[sample_idx] as u64;
                sample_idx += 1;
            }
        }
    }

    Ok(offsets)
}

/// Expand run-length encoded stts entries into per-sample durations.
fn expand_stts(entries: &[SttsEntry], sample_count: usize) -> Vec<u32> {
    let mut durations = Vec::with_capacity(sample_count);
    for entry in entries {
        for _ in 0..entry.sample_count {
            durations.push(entry.sample_delta);
        }
    }
    durations.truncate(sample_count);
    durations
}

/// Expand ctts entries into per-sample composition time offsets.
fn expand_ctts(entries: &[CttsEntry], sample_count: usize) -> Vec<i32> {
    let mut offsets = Vec::with_capacity(sample_count);
    for entry in entries {
        for _ in 0..entry.sample_count {
            offsets.push(entry.sample_offset);
        }
    }
    offsets.truncate(sample_count);
    offsets
}

/// Write a single-sample moof+mdat fragment.
///
/// Returns the total bytes written (moof + mdat).
fn write_frame_fragment<W: Write>(
    writer: &mut W,
    sequence_number: u32,
    track_id: u32,
    base_decode_time: u64,
    sample: &SampleInfo,
    sample_data: &[u8],
) -> Result<u64> {
    // Sample flags per ISOBMFF:
    // sync: 0x02000000 (sample_depends_on=2: does not depend on others)
    // non-sync: 0x01010000 (sample_depends_on=1 + sample_is_non_sync=1)
    let sample_flags: u32 = if sample.is_sync {
        0x02000000
    } else {
        0x01010000
    };

    let has_cts = sample.cts_offset != 0;

    let entry = TrunEntry {
        duration: Some(sample.duration),
        size: Some(sample.size),
        flags: Some(sample_flags),
        cts: if has_cts { Some(sample.cts_offset) } else { None },
    };

    let moof = Moof {
        mfhd: Mfhd { sequence_number },
        traf: vec![Traf {
            tfhd: Tfhd {
                track_id,
                base_data_offset: None,
                sample_description_index: None,
                default_sample_duration: None,
                default_sample_size: None,
                default_sample_flags: None,
            },
            tfdt: Some(Tfdt {
                base_media_decode_time: base_decode_time,
            }),
            trun: vec![Trun {
                data_offset: Some(0), // placeholder
                entries: vec![entry],
            }],
            ..Default::default()
        }],
    };

    // Encode moof to measure its size
    let mut moof_buf = Vec::new();
    moof.encode(&mut moof_buf).map_err(mp4_err)?;
    let moof_size = moof_buf.len();

    // data_offset = offset from start of moof to start of mdat payload
    // = moof_size + 8 (mdat header)
    let data_offset = (moof_size + 8) as i32;

    // Re-encode with correct data_offset
    let mut moof_patched = moof;
    moof_patched.traf[0].trun[0].data_offset = Some(data_offset);
    let mut moof_buf = Vec::new();
    moof_patched.encode(&mut moof_buf).map_err(mp4_err)?;

    // Write moof
    writer.write_all(&moof_buf)?;

    // Write mdat: 4-byte size (big-endian) + 4-byte type + payload
    let mdat_total_size = 8u32 + sample_data.len() as u32;
    writer.write_all(&mdat_total_size.to_be_bytes())?;
    writer.write_all(b"mdat")?;
    writer.write_all(sample_data)?;

    Ok(moof_buf.len() as u64 + mdat_total_size as u64)
}

fn mp4_err(e: mp4_atom::Error) -> Error {
    Error::InvalidMp4(e.to_string())
}

/// Fragment a single track from a flat MP4 into per-frame moof+mdat pairs.
///
/// Writes concatenated fragments to the writer. Returns the number of samples
/// written.
pub fn fragment_track<RS: Read + Seek, W: Write>(
    mut input: RS,
    track_id: u32,
    writer: &mut W,
) -> Result<u32> {
    let moov = read_moov(&mut input)?;

    let trak = moov
        .trak
        .iter()
        .find(|t| t.tkhd.track_id == track_id)
        .ok_or_else(|| Error::InvalidMp4(format!("track {track_id} not found")))?;

    let track_info = extract_track_info(trak)?;
    let sample_count = track_info.samples.len() as u32;
    let mut decode_time: u64 = 0;

    for (i, sample) in track_info.samples.iter().enumerate() {
        // Read sample data from the original file
        input.seek(SeekFrom::Start(sample.file_offset))?;
        let mut data = vec![0u8; sample.size as usize];
        input.read_exact(&mut data)?;

        write_frame_fragment(
            writer,
            (i as u32) + 1,
            track_id,
            decode_time,
            sample,
            &data,
        )?;
        decode_time += sample.duration as u64;
    }

    Ok(sample_count)
}

/// Fragment all tracks from a flat MP4 into per-frame moof+mdat pairs,
/// writing each track's fragments to a separate directory.
///
/// Creates `output_dir/<track_id>/` directories, each containing
/// numbered `.cmaf` files (one per frame).
pub fn fragment_to_directory<RS: Read + Seek>(
    mut input: RS,
    output_dir: &std::path::Path,
) -> Result<FragmentStats> {
    let moov = read_moov(&mut input)?;

    let mut track_ids: Vec<u32> = moov.trak.iter().map(|t| t.tkhd.track_id).collect();
    track_ids.sort();

    let mut stats = FragmentStats {
        tracks: Vec::new(),
    };

    for &track_id in &track_ids {
        let trak = moov
            .trak
            .iter()
            .find(|t| t.tkhd.track_id == track_id)
            .unwrap();

        let track_info = extract_track_info(trak)?;
        let track_dir = output_dir.join(format!("track{}", track_id));
        std::fs::create_dir_all(&track_dir)?;

        let sample_count = track_info.samples.len() as u32;
        let mut decode_time: u64 = 0;
        let mut total_bytes: u64 = 0;

        for (i, sample) in track_info.samples.iter().enumerate() {
            let sample_id = (i as u32) + 1;

            // Read sample data
            input.seek(SeekFrom::Start(sample.file_offset))?;
            let mut data = vec![0u8; sample.size as usize];
            input.read_exact(&mut data)?;

            let mut buf = Vec::new();
            write_frame_fragment(&mut buf, sample_id, track_id, decode_time, sample, &data)?;

            let filename = track_dir.join(format!("{:06}.cmaf", sample_id));
            std::fs::write(&filename, &buf)?;
            total_bytes += buf.len() as u64;
            decode_time += sample.duration as u64;
        }

        let handler = trak.mdia.hdlr.handler;
        stats.tracks.push(TrackStats {
            track_id,
            handler_type: String::from_utf8_lossy(handler.as_ref()).to_string(),
            timescale: trak.mdia.mdhd.timescale,
            sample_count,
            total_bytes,
        });
    }

    Ok(stats)
}

/// Statistics about a fragmentation operation.
pub struct FragmentStats {
    pub tracks: Vec<TrackStats>,
}

pub struct TrackStats {
    pub track_id: u32,
    pub handler_type: String,
    pub timescale: u32,
    pub sample_count: u32,
    pub total_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp4_atom::{Atom, Header, Moof, ReadFrom};
    use std::io::Cursor;

    fn read_fixture(name: &str) -> Vec<u8> {
        let path = format!("samples/fixtures/{}", name);
        std::fs::read(&path)
            .or_else(|_| std::fs::read(format!("samples/{}", name)))
            .unwrap_or_else(|_| panic!("{} must exist for tests", path))
    }

    #[test]
    fn test_fragment_track_h264_aac() {
        let data = read_fixture("h264-aac.mp4");
        let moov = read_moov(&mut Cursor::new(&data)).unwrap();

        for trak in &moov.trak {
            let track_id = trak.tkhd.track_id;
            let mut output = Vec::new();
            let count = fragment_track(Cursor::new(&data), track_id, &mut output).unwrap();
            assert!(count > 0, "track {track_id} should have samples");
            assert!(!output.is_empty(), "track {track_id} should produce output");
        }
    }

    #[test]
    fn test_fragment_produces_moof_mdat_pairs() {
        let data = read_fixture("h264-aac.mp4");
        let moov = read_moov(&mut Cursor::new(&data)).unwrap();
        let track_id = moov.trak[0].tkhd.track_id;

        let mut output = Vec::new();
        fragment_track(Cursor::new(&data), track_id, &mut output).unwrap();

        // Parse the output: should be alternating moof+mdat
        let mut cursor = Cursor::new(&output);
        let mut fragment_count = 0u32;

        while cursor.position() < output.len() as u64 {
            let h1 = <Option<Header> as ReadFrom>::read_from(&mut cursor)
                .unwrap()
                .expect("expected moof header");
            assert_eq!(h1.kind, Moof::KIND, "expected moof, got {:?}", h1.kind);
            let moof_body_size = h1.size.unwrap();
            cursor.seek(SeekFrom::Current(moof_body_size as i64)).unwrap();

            let h2 = <Option<Header> as ReadFrom>::read_from(&mut cursor)
                .unwrap()
                .expect("expected mdat header");
            assert_eq!(
                h2.kind,
                mp4_atom::FourCC::new(b"mdat"),
                "expected mdat, got {:?}",
                h2.kind
            );
            let mdat_body_size = h2.size.unwrap();
            cursor.seek(SeekFrom::Current(mdat_body_size as i64)).unwrap();

            fragment_count += 1;
        }

        assert!(fragment_count > 0, "should have produced fragments");
    }

    #[test]
    fn test_fragment_sample_data_preserved() {
        let data = read_fixture("h264-aac.mp4");
        let moov = read_moov(&mut Cursor::new(&data)).unwrap();
        let track_id = moov.trak[0].tkhd.track_id;

        // Extract expected sample data from original file
        let trak = &moov.trak[0];
        let track_info = extract_track_info(trak).unwrap();

        let mut output = Vec::new();
        fragment_track(Cursor::new(&data), track_id, &mut output).unwrap();

        // For each fragment, verify the mdat payload matches the original sample data
        let mut cursor = Cursor::new(&output);
        for (i, sample) in track_info.samples.iter().enumerate().take(5) {
            // Read original sample
            let mut original = vec![0u8; sample.size as usize];
            let mut input = Cursor::new(&data);
            input.seek(SeekFrom::Start(sample.file_offset)).unwrap();
            input.read_exact(&mut original).unwrap();

            // Skip moof
            let h1 = <Option<Header> as ReadFrom>::read_from(&mut cursor)
                .unwrap()
                .unwrap();
            cursor
                .seek(SeekFrom::Current(h1.size.unwrap() as i64))
                .unwrap();

            // Read mdat
            let h2 = <Option<Header> as ReadFrom>::read_from(&mut cursor)
                .unwrap()
                .unwrap();
            let mdat_size = h2.size.unwrap();
            let mut mdat_payload = vec![0u8; mdat_size];
            Read::read_exact(&mut cursor, &mut mdat_payload).unwrap();

            assert_eq!(
                mdat_payload, original,
                "sample {} data mismatch in track {}",
                i + 1,
                track_id
            );
        }
    }

    #[test]
    fn test_fragment_to_directory() {
        let data = read_fixture("h264-aac.mp4");
        let tmp = TempDir::new();
        let stats = fragment_to_directory(Cursor::new(data), tmp.path()).unwrap();

        assert!(!stats.tracks.is_empty(), "should have at least one track");

        for track in &stats.tracks {
            let track_dir = tmp.path().join(format!("track{}", track.track_id));
            assert!(track_dir.is_dir());

            let files: Vec<_> = std::fs::read_dir(&track_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "cmaf"))
                .collect();
            assert_eq!(
                files.len(),
                track.sample_count as usize,
                "track {} should have {} fragment files",
                track.track_id,
                track.sample_count
            );
        }
    }

    #[test]
    fn test_fragment_all_fixtures() {
        for name in &[
            "h264-aac.mp4",
            "h264-opus.mp4",
            "h264-aac-25fps.mp4",
            "h264-video-only.mp4",
            "opus-audio-only.mp4",
        ] {
            let data = read_fixture(name);
            let moov = read_moov(&mut Cursor::new(&data)).unwrap();

            for trak in &moov.trak {
                let track_id = trak.tkhd.track_id;
                let mut output = Vec::new();
                let count = fragment_track(Cursor::new(&data), track_id, &mut output)
                    .unwrap_or_else(|e| panic!("{name} track {track_id}: {e}"));
                assert!(count > 0, "{name} track {track_id}: no samples");
            }
        }
    }

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("muxl-test-{}", std::process::id()));
            path.push(format!(
                "{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
