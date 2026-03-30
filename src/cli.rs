use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::Path;
use std::process;

fn usage() -> ! {
    eprintln!("Usage: muxl <command> [args...]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  catalog <input.mp4>                       Extract catalog from MP4");
    eprintln!("  init <input.mp4> <output_init.mp4>        Build canonical init segment");
    eprintln!("  segment <input> --dir <output_dir>        Segment fMP4 into directory");
    eprintln!("  segment <input> --archive <output.mp4>    Segment fMP4 into archive file");
    eprintln!("  segment <input> --stdout                  Stream segments to stdout (framed)");
    eprintln!("  concat                                    Concatenate MUXL archives from stdin (CBOR out)");
    eprintln!("  hls <archive.mp4> <output_dir> [--blobs <dir>]  Generate HLS playlists from archive");
    eprintln!();
    eprintln!("  <input> can be a file path or \"-\" for stdin");
    process::exit(1);
}

pub fn cli_main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
    }

    let result = match args[1].as_str() {
        "catalog" => cmd_catalog(&args[2..]),
        "init" => cmd_init(&args[2..]),
        "segment" => cmd_segment(&args[2..]),
        "concat" => cmd_concat(),
        "hls" => cmd_hls(&args[2..]),
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            usage();
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}

fn cmd_catalog(args: &[String]) -> crate::Result<()> {
    if args.len() != 1 {
        eprintln!("Usage: muxl catalog <input.mp4>");
        process::exit(1);
    }
    let data = fs::read(&args[0])?;
    let catalog = crate::catalog_from_mp4(Cursor::new(data))?;

    for (name, v) in &catalog.video {
        eprintln!(
            "video \"{name}\": {} {}x{} (track {}, {} desc bytes)",
            v.codec,
            v.coded_width,
            v.coded_height,
            v.track_id,
            v.description.len()
        );
    }
    for (name, a) in &catalog.audio {
        eprintln!(
            "audio \"{name}\": {} {}Hz {}ch (track {}, {} desc bytes)",
            a.codec,
            a.sample_rate,
            a.number_of_channels,
            a.track_id,
            a.description.len()
        );
    }

    Ok(())
}

fn cmd_init(args: &[String]) -> crate::Result<()> {
    if args.len() != 2 {
        eprintln!("Usage: muxl init <input.mp4> <output_init.mp4>");
        process::exit(1);
    }
    let data = fs::read(&args[0])?;
    let catalog = crate::catalog_from_mp4(Cursor::new(data))?;
    let init = crate::build_init_segment(&catalog)?;
    fs::write(&args[1], &init)?;
    eprintln!("Wrote {} bytes", init.len());
    Ok(())
}

fn cmd_segment(args: &[String]) -> crate::Result<()> {
    if args.len() < 2 {
        eprintln!("Usage: muxl segment <input> --dir <output_dir>");
        eprintln!("       muxl segment <input> --archive <output.mp4>");
        eprintln!("       muxl segment <input> --stdout");
        eprintln!("  <input> can be a file path or \"-\" for stdin");
        process::exit(1);
    }

    let input_path = &args[0];
    let mode = &args[1];

    // Open input: file or stdin
    let mut input: Box<dyn Read> = if input_path == "-" {
        Box::new(io::stdin().lock())
    } else {
        Box::new(fs::File::open(input_path)?)
    };

    match mode.as_str() {
        "--dir" => {
            let output_path = args.get(2).unwrap_or_else(|| {
                eprintln!("Missing output directory");
                process::exit(1);
            });
            cmd_segment_dir(&mut input, output_path)
        }
        "--archive" => {
            let output_path = args.get(2).unwrap_or_else(|| {
                eprintln!("Missing output file");
                process::exit(1);
            });
            cmd_segment_archive(&mut input, output_path)
        }
        "--stdout" => cmd_segment_stdout(&mut input),
        _ => {
            eprintln!("Unknown segment mode: {mode}");
            eprintln!("Use --dir, --archive, or --stdout");
            process::exit(1);
        }
    }
}

fn cmd_segment_dir(input: &mut impl Read, output_dir: &str) -> crate::Result<()> {
    let output_dir = std::path::Path::new(output_dir);
    fs::create_dir_all(output_dir)?;

    let catalog = crate::segment_fmp4(input, |gop| {
        for (&track_id, data) in &gop.tracks {
            let track_dir = output_dir.join(format!("track{}", track_id));
            fs::create_dir_all(&track_dir)?;
            let filename = track_dir.join(format!("segment_{:04}.m4s", gop.number));
            fs::write(&filename, data)?;
            eprintln!(
                "track {} segment {:4}: {} bytes",
                track_id, gop.number, data.len()
            );
        }
        Ok(())
    })?;

    // Write init segment
    let init = crate::build_init_segment(&catalog)?;
    let init_path = output_dir.join("init.mp4");
    fs::write(&init_path, &init)?;
    eprintln!("init: {} bytes", init.len());

    Ok(())
}

/// Stream segments to stdout as CBOR (DRISL) events.
///
/// Each event is a separate CBOR value in the stream:
///   {"type": "init", "data": <bstr>}
///   {"type": "segment", "number": <uint>, "data": <bstr>}
///
/// Uses the push-based segmenter so init is emitted first (before segments).
fn cmd_segment_stdout(input: &mut impl Read) -> crate::Result<()> {
    let mut stdout = io::stdout().lock();
    let mut buf = [0u8; 64 * 1024];
    let mut segmenter = crate::Segmenter::new();

    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for event in segmenter.feed(&buf[..n])? {
            write_cbor_event(&mut stdout, &event)?;
        }
    }
    for event in segmenter.flush()? {
        write_cbor_event(&mut stdout, &event)?;
    }
    Ok(())
}

fn write_cbor_event(w: &mut impl io::Write, event: &crate::SegmenterEvent) -> crate::Result<()> {
    let cbor_event = crate::cbor::CborEvent::from_event(event);
    dasl::drisl::to_writer(&mut *w, &cbor_event)
        .map_err(|e| crate::Error::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;
    w.flush()?;
    match event {
        crate::SegmenterEvent::InitSegment { data, .. } => {
            eprintln!("init: {} bytes", data.len());
        }
        crate::SegmenterEvent::Segment(gop) => {
            let total: usize = gop.tracks.values().map(|d| d.len()).sum();
            eprintln!(
                "segment {}: {} tracks, {} bytes",
                gop.number,
                gop.tracks.len(),
                total
            );
        }
    }
    Ok(())
}

/// Concatenate MUXL archives from stdin, emit CBOR events to stdout.
///
/// Reads concatenated MUXL fMP4 archives from stdin. Emits init events only
/// when the catalog changes between archives. UUID atoms delimit segments
/// and are passed through in the segment data.
fn cmd_concat() -> crate::Result<()> {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut buf = [0u8; 64 * 1024];
    let mut concat = crate::Concatenator::new();

    loop {
        let n = stdin.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for event in concat.feed(&buf[..n])? {
            write_cbor_event(&mut stdout, &event)?;
        }
    }
    for event in concat.flush()? {
        write_cbor_event(&mut stdout, &event)?;
    }
    Ok(())
}

fn cmd_hls(args: &[String]) -> crate::Result<()> {
    if args.len() < 2 {
        eprintln!("Usage: muxl hls <archive.mp4> <output_dir> [--blobs <blobs_dir>]");
        process::exit(1);
    }
    let archive_path = &args[0];
    let output_dir = Path::new(&args[1]);

    // Parse optional --blobs flag
    let mut blobs_dir: Option<&Path> = None;
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--blobs" {
            blobs_dir = Some(Path::new(args.get(i + 1).unwrap_or_else(|| {
                eprintln!("Missing blobs directory after --blobs");
                process::exit(1);
            })));
            i += 2;
        } else {
            i += 1;
        }
    }

    let data = fs::read(archive_path)?;

    // Compute BLAKE3 CID and determine archive filename for playlists
    let archive_filename = if let Some(bd) = blobs_dir {
        fs::create_dir_all(bd)?;
        let cid = bdasl_cid(&data);
        let blob_name = format!("{cid}.mp4");
        let blob_path = bd.join(&blob_name);
        if !blob_path.exists() {
            // Try hardlink first, fall back to copy
            if fs::hard_link(archive_path, &blob_path).is_err() {
                fs::copy(archive_path, &blob_path)?;
            }
        }
        // Compute relative path from output_dir to blob
        let rel = pathdiff(bd, output_dir)
            .map(|p| p.join(&blob_name))
            .unwrap_or_else(|| blob_path.to_path_buf());
        eprintln!("blob: {cid} ({} bytes)", data.len());
        rel.to_string_lossy().into_owned()
    } else {
        Path::new(archive_path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };

    let mut cursor = Cursor::new(&data);

    // Extract catalog and build per-track init segments
    let catalog = crate::catalog_from_mp4(Cursor::new(&data))?;
    let track_inits = crate::init::build_track_init_segments(&catalog)?;

    // Compute init segment size (ftyp+moov) by finding the first moof
    let init_size = find_first_moof_offset(&data);

    // Segment the archive to get per-track, per-GOP durations and sample counts
    let mut gops: Vec<crate::GopSegment> = Vec::new();
    crate::segment_fmp4(&mut cursor, |gop| {
        gops.push(gop);
        Ok(())
    })?;

    // Build a track_id → timescale map
    let mut timescales: BTreeMap<u32, u32> = BTreeMap::new();
    for v in catalog.video.values() {
        timescales.insert(v.track_id, v.timescale);
    }
    for a in catalog.audio.values() {
        timescales.insert(a.track_id, a.timescale);
    }

    // Collect all track IDs
    let mut track_ids: Vec<u32> = gops
        .iter()
        .flat_map(|g| g.tracks.keys().copied())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    track_ids.sort();

    // Compute byte offsets for each track's GOPs in the archive.
    // Archive layout: init + [track1 gop1, gop2, ...] + [track2 gop1, gop2, ...]
    let mut track_gop_offsets: BTreeMap<u32, Vec<(u64, u64, u64, u32)>> = BTreeMap::new(); // (offset, size, duration_ticks, sample_count)
    let mut offset = init_size as u64;
    for &tid in &track_ids {
        let mut entries = Vec::new();
        for gop in &gops {
            if let Some(seg_data) = gop.tracks.get(&tid) {
                let size = seg_data.len() as u64;
                let dur = gop.durations.get(&tid).copied().unwrap_or(0);
                let sc = gop.sample_counts.get(&tid).copied().unwrap_or(0);
                entries.push((offset, size, dur, sc));
                offset += size;
            }
        }
        track_gop_offsets.insert(tid, entries);
    }

    fs::create_dir_all(output_dir)?;

    // Write per-track init segments
    let mut track_init_filenames: BTreeMap<u32, String> = BTreeMap::new();
    let mut track_init_cids: BTreeMap<u32, String> = BTreeMap::new();
    for (&tid, init_data) in &track_inits {
        let init_cid = bdasl_cid(init_data);
        track_init_cids.insert(tid, init_cid.clone());

        if let Some(bd) = blobs_dir {
            let blob_path = bd.join(format!("{init_cid}.mp4"));
            if !blob_path.exists() {
                fs::write(&blob_path, init_data)?;
            }
        }

        let filename = format!("init-{tid}.mp4");
        fs::write(output_dir.join(&filename), init_data)?;
        track_init_filenames.insert(tid, filename);
    }

    // Build track type map
    let mut track_types: BTreeMap<u32, &str> = BTreeMap::new();
    for v in catalog.video.values() {
        track_types.insert(v.track_id, "video");
    }
    for a in catalog.audio.values() {
        track_types.insert(a.track_id, "audio");
    }

    // Generate master playlist
    let mut master = String::new();
    master.push_str("#EXTM3U\n#EXT-X-VERSION:6\n\n");

    for a in catalog.audio.values() {
        let tid = a.track_id;
        master.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"audio{tid}\",\
             DEFAULT=YES,AUTOSELECT=YES,CHANNELS=\"{}\",URI=\"audio-{tid}.m3u8\"\n",
            a.number_of_channels
        ));
    }
    master.push('\n');

    for v in catalog.video.values() {
        let tid = v.track_id;
        let entries = track_gop_offsets.get(&tid).unwrap_or(&Vec::new()).clone();
        let total_bytes: u64 = entries.iter().map(|(_, s, _, _)| s).sum();
        let total_ticks: u64 = entries.iter().map(|(_, _, d, _)| d).sum();
        let ts = v.timescale as f64;
        let total_dur = total_ticks as f64 / ts;
        let bandwidth = if total_dur > 0.0 {
            (total_bytes as f64 * 8.0 / total_dur) as u64
        } else {
            0
        };
        let total_samples: u32 = entries.iter().map(|(_, _, _, sc)| sc).sum();
        let frame_rate = if total_dur > 0.0 {
            total_samples as f64 / total_dur
        } else {
            0.0
        };

        master.push_str(&format!(
            "#EXT-X-STREAM-INF:AUDIO=\"audio\",BANDWIDTH={bandwidth},\
             CODECS=\"{},{}\",RESOLUTION={}x{},FRAME-RATE={frame_rate:.3}\n",
            v.codec,
            catalog.audio.values().next().map(|a| a.codec.as_str()).unwrap_or("mp4a.40.2"),
            v.coded_width,
            v.coded_height,
        ));
        master.push_str(&format!("video-{tid}.m3u8\n"));
    }

    fs::write(output_dir.join("master.m3u8"), &master)?;

    // Generate per-track media playlists
    for &tid in &track_ids {
        let entries = track_gop_offsets.get(&tid).unwrap_or(&Vec::new()).clone();
        let ts = *timescales.get(&tid).unwrap_or(&1) as f64;
        let track_type = *track_types.get(&tid).unwrap_or(&"unknown");
        let init_filename = track_init_filenames
            .get(&tid)
            .cloned()
            .unwrap_or_else(|| archive_filename.clone());

        let max_dur: f64 = entries
            .iter()
            .map(|(_, _, d, _)| *d as f64 / ts)
            .fold(0.0, f64::max);
        let target_dur = (max_dur.ceil() as u64).max(1);

        let mut playlist = String::new();
        playlist.push_str("#EXTM3U\n");
        playlist.push_str("#EXT-X-VERSION:6\n");
        playlist.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");
        playlist.push_str("#EXT-X-INDEPENDENT-SEGMENTS\n");
        playlist.push_str(&format!("#EXT-X-TARGETDURATION:{target_dur}\n"));
        playlist.push_str("#EXT-X-MEDIA-SEQUENCE:0\n");
        playlist.push_str(&format!("#EXT-X-MAP:URI=\"{init_filename}\"\n\n"));

        for (seg_offset, seg_size, dur_ticks, _) in &entries {
            let dur_sec = *dur_ticks as f64 / ts;
            playlist.push_str(&format!("#EXTINF:{dur_sec:.6},\n"));
            playlist.push_str(&format!(
                "#EXT-X-BYTERANGE:{seg_size}@{seg_offset}\n"
            ));
            playlist.push_str(&archive_filename);
            playlist.push('\n');
        }

        playlist.push_str("#EXT-X-ENDLIST\n");
        fs::write(
            output_dir.join(format!("{track_type}-{tid}.m3u8")),
            &playlist,
        )?;
    }

    // Write metadata JSON for programmatic consumers (e.g. Cloudflare worker)
    let blob_cid = bdasl_cid(&data);
    let mut meta_tracks = serde_json::Map::new();
    for &tid in &track_ids {
        let entries = track_gop_offsets.get(&tid).unwrap_or(&Vec::new()).clone();
        let ts = *timescales.get(&tid).unwrap_or(&1);
        let track_type = *track_types.get(&tid).unwrap_or(&"unknown");

        let segments: Vec<serde_json::Value> = entries
            .iter()
            .map(|(offset, size, dur_ticks, sample_count)| {
                serde_json::json!({
                    "offset": offset,
                    "size": size,
                    "durationTicks": dur_ticks,
                    "sampleCount": sample_count,
                })
            })
            .collect();

        let init_cid = track_init_cids.get(&tid).cloned();

        let track_info = match track_type {
            "video" => {
                let v = catalog.video.values().find(|v| v.track_id == tid);
                serde_json::json!({
                    "type": "video",
                    "codec": v.map(|v| v.codec.as_str()).unwrap_or(""),
                    "width": v.map(|v| v.coded_width).unwrap_or(0),
                    "height": v.map(|v| v.coded_height).unwrap_or(0),
                    "timescale": ts,
                    "initCid": init_cid,
                    "segments": segments,
                })
            }
            _ => {
                let a = catalog.audio.values().find(|a| a.track_id == tid);
                serde_json::json!({
                    "type": "audio",
                    "codec": a.map(|a| a.codec.as_str()).unwrap_or(""),
                    "channels": a.map(|a| a.number_of_channels).unwrap_or(0),
                    "sampleRate": a.map(|a| a.sample_rate).unwrap_or(0),
                    "timescale": ts,
                    "initCid": init_cid,
                    "segments": segments,
                })
            }
        };
        meta_tracks.insert(tid.to_string(), track_info);
    }

    let metadata = serde_json::json!({
        "blobCid": blob_cid,
        "blobSize": data.len(),
        "tracks": meta_tracks,
    });
    fs::write(
        output_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata).unwrap_or_default(),
    )?;

    eprintln!(
        "HLS: master.m3u8 + {} track playlists, {} GOPs, blob {}",
        track_ids.len(),
        gops.len(),
        blob_cid,
    );
    Ok(())
}

/// Compute a BDASL CID: CIDv1, raw codec (0x55), BLAKE3 (0x1e), base32lower.
fn bdasl_cid(data: &[u8]) -> String {
    let digest = blake3::hash(data);
    // CID binary: version(1) + codec(0x55=raw) + hash_fn(0x1e=blake3) + hash_len(0x20) + digest
    let mut cid_bytes = vec![0x01, 0x55, 0x1e, 0x20];
    cid_bytes.extend_from_slice(digest.as_bytes());
    let mut out = String::from("b");
    base32_lower_encode(&cid_bytes, &mut out);
    out
}

fn base32_lower_encode(data: &[u8], out: &mut String) {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut buffer: u64 = 0;
    let mut bits = 0;
    for &byte in data {
        buffer = (buffer << 8) | byte as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1F) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1F) as usize] as char);
    }
}

/// Compute a relative path from `base` to `target`.
fn pathdiff(target: &Path, base: &Path) -> Option<std::path::PathBuf> {
    let target = fs::canonicalize(target).ok()?;
    let base = fs::canonicalize(base).ok()?;

    let mut target_parts = target.components().peekable();
    let mut base_parts = base.components().peekable();

    // Skip common prefix
    while let (Some(t), Some(b)) = (target_parts.peek(), base_parts.peek()) {
        if t == b {
            target_parts.next();
            base_parts.next();
        } else {
            break;
        }
    }

    let mut result = std::path::PathBuf::new();
    for _ in base_parts {
        result.push("..");
    }
    for part in target_parts {
        result.push(part);
    }
    Some(result)
}

/// Find the byte offset of the first moof box (= end of init segment).
fn find_first_moof_offset(data: &[u8]) -> usize {
    let mut pos = 0;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
            as usize;
        let box_type = &data[pos + 4..pos + 8];
        if box_type == b"moof" {
            return pos;
        }
        if size == 0 || pos + size > data.len() {
            break;
        }
        pos += size;
    }
    pos
}

fn cmd_segment_archive(input: &mut impl Read, output_path: &str) -> crate::Result<()> {
    let mut gops = Vec::new();

    let catalog = crate::segment_fmp4(input, |gop| {
        let total: usize = gop.tracks.values().map(|d| d.len()).sum();
        eprintln!(
            "segment {:4}: {} tracks, {} bytes",
            gop.number,
            gop.tracks.len(),
            total
        );
        gops.push(gop);
        Ok(())
    })?;

    // Collect track IDs in order
    let mut track_ids: Vec<u32> = gops
        .iter()
        .flat_map(|g| g.tracks.keys().copied())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    track_ids.sort();

    // Build per-track archive: init + [all track1 segments] + [all track2 segments]
    let init = crate::build_init_segment(&catalog)?;
    let mut archive = init;
    for &tid in &track_ids {
        for gop in &gops {
            if let Some(data) = gop.tracks.get(&tid) {
                archive.extend_from_slice(data);
            }
        }
    }

    fs::write(output_path, &archive)?;
    eprintln!(
        "archive: {} bytes ({} GOPs, {} tracks)",
        archive.len(),
        gops.len(),
        track_ids.len()
    );

    Ok(())
}
