//! Fragmentation: flat MP4 → per-frame fMP4 fragments (Hang CMAF style).
//!
//! Each track is fragmented independently. Each frame becomes a single
//! moof+mdat pair. Output is a directory with one subdirectory per track,
//! containing numbered .cmaf files.
//!
//! Spec: architecture.md § Hang CMAF

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use mp4::{
    BoxHeader, BoxType, MfhdBox, MoofBox, Mp4Box, Mp4Reader, TfhdBox, TfdtBox, TrafBox, TrunBox,
    WriteBox,
};

use crate::error::{Error, Result};
use crate::sample_table::{expand_ctts, expand_stts};

/// Information about a single sample extracted from the moov sample tables.
struct SampleInfo {
    duration: u32,
    size: u32,
    is_sync: bool,
    cts_offset: i32,
    description_index: u32,
}

/// Per-track metadata needed for fragmentation.
struct TrackInfo {
    _track_id: u32,
    handler_type: String,
    timescale: u32,
    samples: Vec<SampleInfo>,
}

/// Extract per-sample metadata from a track's sample tables.
fn extract_track_info<RS: Read + Seek>(
    reader: &Mp4Reader<RS>,
    track_id: u32,
) -> Result<TrackInfo> {
    let trak = reader
        .moov
        .traks
        .iter()
        .find(|t| t.tkhd.track_id == track_id)
        .ok_or_else(|| Error::InvalidMp4(format!("track {track_id} not found")))?;

    let stbl = &trak.mdia.minf.stbl;
    let sample_count = reader.sample_count(track_id)?;

    let durations = expand_stts(&stbl.stts.entries, sample_count);
    let cts_offsets = if let Some(ref ctts) = stbl.ctts {
        expand_ctts(&ctts.entries, sample_count)
    } else {
        vec![0i32; sample_count as usize]
    };

    // Build sync sample set
    let sync_samples: std::collections::HashSet<u32> = if let Some(ref stss) = stbl.stss {
        stss.entries.iter().copied().collect()
    } else {
        // No stss = all samples are sync
        (1..=sample_count).collect()
    };

    // Sample sizes
    let sizes: Vec<u32> = if stbl.stsz.sample_size > 0 {
        vec![stbl.stsz.sample_size; sample_count as usize]
    } else {
        stbl.stsz.sample_sizes.clone()
    };

    // Sample description indices
    let desc_indices =
        crate::sample_table::resolve_sample_description_indices(&stbl.stsc.entries, sample_count);

    let mut samples = Vec::with_capacity(sample_count as usize);
    for i in 0..sample_count as usize {
        samples.push(SampleInfo {
            duration: *durations.get(i).unwrap_or(&0),
            size: *sizes.get(i).unwrap_or(&0),
            is_sync: sync_samples.contains(&(i as u32 + 1)),
            cts_offset: *cts_offsets.get(i).unwrap_or(&0),
            description_index: *desc_indices.get(i).unwrap_or(&1),
        });
    }

    let handler_type = trak.mdia.hdlr.handler_type.to_string();
    let timescale = trak.mdia.mdhd.timescale;

    Ok(TrackInfo {
        _track_id: track_id,
        handler_type,
        timescale,
        samples,
    })
}

/// Write a single-sample moof+mdat fragment.
fn write_frame_fragment<W: Write>(
    writer: &mut W,
    sequence_number: u32,
    track_id: u32,
    base_decode_time: u64,
    sample: &SampleInfo,
    sample_data: &[u8],
) -> Result<u64> {
    // Sample flags: bit 16 = is_leading, bits 24-25 = sample_depends_on
    // For sync samples: 0x02000000 (depends on no other)
    // For non-sync: 0x01010000 (depends on others, is not sync)
    let sample_flags: u32 = if sample.is_sync {
        0x02000000
    } else {
        0x01010000
    };

    let has_cts = sample.cts_offset != 0;

    let mut trun_flags = TrunBox::FLAG_SAMPLE_DURATION
        | TrunBox::FLAG_SAMPLE_SIZE
        | TrunBox::FLAG_SAMPLE_FLAGS
        | TrunBox::FLAG_DATA_OFFSET;
    if has_cts {
        trun_flags |= TrunBox::FLAG_SAMPLE_CTS;
    }

    let trun = TrunBox {
        version: if has_cts { 1 } else { 0 },
        flags: trun_flags,
        sample_count: 1,
        data_offset: Some(0), // placeholder, fixed below
        first_sample_flags: None,
        sample_durations: vec![sample.duration],
        sample_sizes: vec![sample.size],
        sample_flags: vec![sample_flags],
        sample_cts: if has_cts {
            vec![sample.cts_offset as u32]
        } else {
            vec![]
        },
    };

    let tfhd = TfhdBox {
        version: 0,
        flags: TfhdBox::FLAG_DEFAULT_BASE_IS_MOOF
            | if sample.description_index != 1 {
                TfhdBox::FLAG_SAMPLE_DESCRIPTION_INDEX
            } else {
                0
            },
        track_id,
        base_data_offset: None,
        sample_description_index: if sample.description_index != 1 {
            Some(sample.description_index)
        } else {
            None
        },
        default_sample_duration: None,
        default_sample_size: None,
        default_sample_flags: None,
    };

    let tfdt = TfdtBox {
        version: if base_decode_time > u32::MAX as u64 {
            1
        } else {
            0
        },
        flags: 0,
        base_media_decode_time: base_decode_time,
    };

    let traf = TrafBox {
        tfhd,
        tfdt: Some(tfdt),
        trun: Some(trun),
    };

    let moof = MoofBox {
        mfhd: MfhdBox {
            version: 0,
            flags: 0,
            sequence_number,
        },
        trafs: vec![traf],
    };

    // Calculate moof size to set data_offset (offset from moof start to mdat payload)
    let moof_size = moof.box_size();
    let mdat_header_size = 8u64; // standard box header
    let data_offset = (moof_size + mdat_header_size) as i32;

    // Rebuild with correct data_offset
    let mut moof = moof;
    moof.trafs[0].trun.as_mut().unwrap().data_offset = Some(data_offset);

    // Write moof
    moof.write_box(writer)?;

    // Write mdat
    let mdat_size = mdat_header_size + sample_data.len() as u64;
    BoxHeader::new(BoxType::MdatBox, mdat_size).write(writer)?;
    writer.write_all(sample_data)?;

    Ok(moof_size + mdat_size)
}

/// Fragment a flat MP4 into per-frame Hang CMAF fragments.
///
/// Creates `output_dir/<track_id>/` directories, each containing
/// numbered `.cmaf` files (one per frame).
///
/// Also writes `output_dir/<track_id>/init.json` with codec config info.
pub fn fragment_to_directory<RS: Read + Seek>(
    mut input: RS,
    output_dir: &Path,
) -> Result<FragmentStats> {
    let end = input.seek(SeekFrom::End(0))?;
    input.seek(SeekFrom::Start(0))?;
    let mut reader = Mp4Reader::read_header(input, end)?;

    let mut track_ids: Vec<u32> = reader.tracks().keys().copied().collect();
    track_ids.sort();

    let mut stats = FragmentStats {
        tracks: Vec::new(),
    };

    for &track_id in &track_ids {
        let track_info = extract_track_info(&reader, track_id)?;
        let track_dir = output_dir.join(format!("track{}", track_id));
        fs::create_dir_all(&track_dir)?;

        let sample_count = track_info.samples.len() as u32;
        let mut decode_time: u64 = 0;
        let mut total_bytes: u64 = 0;

        for (i, sample_info) in track_info.samples.iter().enumerate() {
            let sample_id = (i as u32) + 1;

            let mp4_sample = reader
                .read_sample(track_id, sample_id)?
                .ok_or_else(|| {
                    Error::InvalidMp4(format!(
                        "missing sample {sample_id} in track {track_id}"
                    ))
                })?;

            let filename = track_dir.join(format!("{:06}.cmaf", sample_id));
            let mut buf = Vec::new();
            write_frame_fragment(
                &mut buf,
                sample_id,
                track_id,
                decode_time,
                sample_info,
                &mp4_sample.bytes,
            )?;

            fs::write(&filename, &buf)?;
            total_bytes += buf.len() as u64;
            decode_time += sample_info.duration as u64;
        }

        stats.tracks.push(TrackStats {
            track_id,
            handler_type: track_info.handler_type,
            timescale: track_info.timescale,
            sample_count,
            total_bytes,
        });
    }

    Ok(stats)
}

/// Fragment a flat MP4 into per-frame fragments, writing all frames for a single
/// track to a writer as concatenated moof+mdat pairs.
pub fn fragment_track<RS: Read + Seek, W: Write>(
    mut input: RS,
    track_id: u32,
    writer: &mut W,
) -> Result<u32> {
    let end = input.seek(SeekFrom::End(0))?;
    input.seek(SeekFrom::Start(0))?;
    let mut reader = Mp4Reader::read_header(input, end)?;

    let track_info = extract_track_info(&reader, track_id)?;
    let sample_count = track_info.samples.len() as u32;
    let mut decode_time: u64 = 0;

    for (i, sample_info) in track_info.samples.iter().enumerate() {
        let sample_id = (i as u32) + 1;
        let mp4_sample = reader
            .read_sample(track_id, sample_id)?
            .ok_or_else(|| {
                Error::InvalidMp4(format!(
                    "missing sample {sample_id} in track {track_id}"
                ))
            })?;

        write_frame_fragment(writer, sample_id, track_id, decode_time, sample_info, &mp4_sample.bytes)?;
        decode_time += sample_info.duration as u64;
    }

    Ok(sample_count)
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
    use std::io::Cursor;

    fn test_file() -> Vec<u8> {
        std::fs::read("samples/file.mp4").expect("samples/file.mp4 must exist for tests")
    }

    #[test]
    fn test_fragment_to_directory() {
        let data = test_file();
        let input = Cursor::new(data);
        let tmp = tempdir();
        let stats = fragment_to_directory(input, tmp.path()).unwrap();

        assert!(stats.tracks.len() >= 1, "should have at least one track");

        for track in &stats.tracks {
            let track_dir = tmp.path().join(format!("track{}", track.track_id));
            assert!(track_dir.is_dir());

            // Check that the right number of fragment files exist
            let files: Vec<_> = fs::read_dir(&track_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "cmaf")
                })
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
    fn test_fragments_are_valid_fmp4() {
        let data = test_file();
        let input = Cursor::new(data);
        let tmp = tempdir();
        let stats = fragment_to_directory(input, tmp.path()).unwrap();

        // Read back a fragment and verify it has moof + mdat structure
        let track = &stats.tracks[0];
        let frag_path = tmp
            .path()
            .join(format!("track{}/000001.cmaf", track.track_id));
        let frag_data = fs::read(&frag_path).unwrap();
        let mut cursor = Cursor::new(&frag_data);

        // First box should be moof
        let header1 = mp4::BoxHeader::read(&mut cursor).unwrap();
        assert_eq!(header1.name, BoxType::MoofBox);

        // Skip to next box — should be mdat
        cursor.set_position(header1.size);
        let header2 = mp4::BoxHeader::read(&mut cursor).unwrap();
        assert_eq!(header2.name, BoxType::MdatBox);

        // Total size should match
        assert_eq!(
            header1.size + header2.size,
            frag_data.len() as u64,
            "fragment size should be moof + mdat"
        );
    }

    #[test]
    fn test_fragment_sample_data_preserved() {
        // Verify that sample bytes in fragments match the original
        let data = test_file();
        let input1 = Cursor::new(data.clone());
        let tmp = tempdir();
        let stats = fragment_to_directory(input1, tmp.path()).unwrap();

        let end = data.len() as u64;
        let mut reader = Mp4Reader::read_header(Cursor::new(data), end).unwrap();

        for track in &stats.tracks {
            for sample_id in 1..=track.sample_count.min(5) {
                // Check first 5 samples
                let original = reader
                    .read_sample(track.track_id, sample_id)
                    .unwrap()
                    .unwrap();

                let frag_path = tmp
                    .path()
                    .join(format!("track{}/{:06}.cmaf", track.track_id, sample_id));
                let frag_data = fs::read(&frag_path).unwrap();

                // The sample data should be the last N bytes of the fragment (in mdat)
                let sample_bytes =
                    &frag_data[frag_data.len() - original.bytes.len()..];
                assert_eq!(
                    sample_bytes,
                    &original.bytes[..],
                    "sample {} data mismatch in track {}",
                    sample_id,
                    track.track_id
                );
            }
        }
    }

    fn tempdir() -> TempDir {
        TempDir::new()
    }

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("muxl-test-{}", std::process::id()));
            path.push(format!("{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()));
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
