use mp4::TrakBox;

use crate::error::{Error, Result};

// Canonical media timescales per handler type.
// Spec: canonical-form.md § mdhd
pub const CANONICAL_VIDEO_TIMESCALE: u32 = 60000;
pub const CANONICAL_AUDIO_TIMESCALE: u32 = 48000;

/// Rescale a value from one timescale to another. Returns None if lossy.
pub fn rescale_exact(value: u64, old_ts: u64, new_ts: u64) -> Option<u64> {
    if old_ts == new_ts || old_ts == 0 {
        return Some(value);
    }
    let numerator = value * new_ts;
    if numerator % old_ts != 0 {
        None
    } else {
        Some(numerator / old_ts)
    }
}

pub fn rescale_exact_i32(value: i32, old_ts: u64, new_ts: u64) -> Option<i32> {
    if old_ts == new_ts || old_ts == 0 {
        return Some(value);
    }
    let abs = value.unsigned_abs() as u64;
    let scaled = rescale_exact(abs, old_ts, new_ts)?;
    let result = i32::try_from(scaled).ok()?;
    Some(if value < 0 { -result } else { result })
}

/// Attempt to rescale a track's media timescale to the canonical value.
/// Returns Ok(true) if rescaled, Ok(false) if already canonical, Err if lossy.
pub fn rescale_track_timescale(trak: &mut TrakBox, canonical_ts: u32) -> Result<bool> {
    let old_ts = trak.mdia.mdhd.timescale as u64;
    let new_ts = canonical_ts as u64;
    if old_ts == new_ts {
        return Ok(false);
    }

    // Check all stts deltas
    for entry in &trak.mdia.minf.stbl.stts.entries {
        if rescale_exact(entry.sample_delta as u64, old_ts, new_ts).is_none() {
            return Err(Error::InvalidMp4(format!(
                "cannot losslessly rescale stts delta {} from timescale {old_ts} to {new_ts}",
                entry.sample_delta
            )));
        }
    }

    // Check all ctts offsets
    if let Some(ref ctts) = trak.mdia.minf.stbl.ctts {
        for entry in &ctts.entries {
            if rescale_exact_i32(entry.sample_offset, old_ts, new_ts).is_none() {
                return Err(Error::InvalidMp4(format!(
                    "cannot losslessly rescale ctts offset {} from timescale {old_ts} to {new_ts}",
                    entry.sample_offset
                )));
            }
        }
    }

    // Check mdhd duration
    if rescale_exact(trak.mdia.mdhd.duration, old_ts, new_ts).is_none() {
        return Err(Error::InvalidMp4(format!(
            "cannot losslessly rescale mdhd duration {} from timescale {old_ts} to {new_ts}",
            trak.mdia.mdhd.duration
        )));
    }

    // Check elst media_time entries
    if let Some(ref edts) = trak.edts {
        if let Some(ref elst) = edts.elst {
            for entry in &elst.entries {
                if entry.media_time != u32::MAX as u64 && entry.media_time != u64::MAX {
                    if rescale_exact(entry.media_time, old_ts, new_ts).is_none() {
                        return Err(Error::InvalidMp4(format!(
                            "cannot losslessly rescale elst media_time {} from timescale {old_ts} to {new_ts}",
                            entry.media_time
                        )));
                    }
                }
            }
        }
    }

    // All checks passed — apply the rescaling.
    for entry in &mut trak.mdia.minf.stbl.stts.entries {
        entry.sample_delta =
            rescale_exact(entry.sample_delta as u64, old_ts, new_ts).unwrap() as u32;
    }
    if let Some(ref mut ctts) = trak.mdia.minf.stbl.ctts {
        for entry in &mut ctts.entries {
            entry.sample_offset =
                rescale_exact_i32(entry.sample_offset, old_ts, new_ts).unwrap();
        }
    }
    trak.mdia.mdhd.duration =
        rescale_exact(trak.mdia.mdhd.duration, old_ts, new_ts).unwrap();
    trak.mdia.mdhd.timescale = canonical_ts;

    if let Some(ref mut edts) = trak.edts {
        if let Some(ref mut elst) = edts.elst {
            for entry in &mut elst.entries {
                if entry.media_time != u32::MAX as u64 && entry.media_time != u64::MAX {
                    entry.media_time =
                        rescale_exact(entry.media_time, old_ts, new_ts).unwrap();
                }
            }
        }
    }

    Ok(true)
}

/// Return the canonical timescale for a handler type, if any.
pub fn canonical_timescale_for_handler(handler_type: &str) -> Option<u32> {
    match handler_type {
        "vide" => Some(CANONICAL_VIDEO_TIMESCALE),
        "soun" => Some(CANONICAL_AUDIO_TIMESCALE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rescale_exact_identity() {
        assert_eq!(rescale_exact(100, 1000, 1000), Some(100));
    }

    #[test]
    fn test_rescale_exact_simple() {
        assert_eq!(rescale_exact(100, 1000, 2000), Some(200));
    }

    #[test]
    fn test_rescale_exact_lossy() {
        assert_eq!(rescale_exact(100, 1000, 3000), Some(300));
        assert_eq!(rescale_exact(1, 3, 2), None); // 2/3 is not an integer
    }

    #[test]
    fn test_rescale_exact_zero_timescale() {
        assert_eq!(rescale_exact(100, 0, 1000), Some(100));
    }

    #[test]
    fn test_rescale_exact_i32_negative() {
        assert_eq!(rescale_exact_i32(-100, 1000, 2000), Some(-200));
    }

    #[test]
    fn test_rescale_video_timescale() {
        // gstreamer 6000 → canonical 60000 (10x)
        assert_eq!(rescale_exact(102, 6000, 60000), Some(1020));
        assert_eq!(rescale_exact(96, 6000, 60000), Some(960));
    }
}
