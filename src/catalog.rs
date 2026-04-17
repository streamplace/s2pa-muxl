//! Catalog types for track configuration metadata.
//!
//! These types capture the codec configuration needed to reconstruct MP4
//! init segments (ftyp+moov) from out-of-band metadata. They align with
//! the Hang catalog schema (WebCodecs-style codec descriptions) but are
//! serialization-format-agnostic at this layer.
//!
//! Spec: architecture.md § Hang CMAF

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Complete catalog describing all tracks in a presentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    /// Video tracks, keyed by rendition name.
    pub video: BTreeMap<String, VideoTrackConfig>,
    /// Audio tracks, keyed by rendition name.
    pub audio: BTreeMap<String, AudioTrackConfig>,
}

/// A single entry in a track's edit list (elst).
///
/// Encodes a piece of the presentation timeline. `media_time = -1` marks an
/// empty edit (a gap — the track presents nothing for `segment_duration`
/// ticks of the movie timescale, useful for aligning tracks that start at
/// different times, as editors like LosslessCut do).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditEntry {
    /// Duration of this edit in the movie timescale (`mvhd.timescale`).
    pub segment_duration: u64,
    /// Start time within the track's media timescale, or `-1` for an empty edit.
    pub media_time: i64,
    /// Playback rate integer part (typically 1).
    pub media_rate: u16,
    /// Playback rate fractional part (typically 0).
    pub media_rate_fraction: u16,
}

/// Configuration for a video track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoTrackConfig {
    /// WebCodecs codec string, e.g. "avc1.64001f", "av01.0.08M.08".
    pub codec: String,
    /// Raw codec-specific description bytes (avcC record for H.264, av1C for AV1).
    /// This is the WebCodecs VideoDecoderConfig.description content.
    #[serde(with = "serde_bytes")]
    pub description: Vec<u8>,
    /// Coded pixel width.
    pub coded_width: u32,
    /// Coded pixel height.
    pub coded_height: u32,
    /// Track ID for the MP4 container.
    pub track_id: u32,
    /// Media timescale (ticks per second). Sample durations in fragments are
    /// expressed in this timescale. Matches Hang catalog container.timescale.
    pub timescale: u32,
    /// Optional edit list, preserved from the source for presentation-timeline
    /// fidelity (e.g. empty edits for A/V alignment from clip editors).
    /// Serialization is skipped — this is carried through file-to-file
    /// transforms but not across the wire catalog.
    #[serde(skip)]
    pub edits: Option<Vec<EditEntry>>,
}

/// Configuration for an audio track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioTrackConfig {
    /// WebCodecs codec string, e.g. "opus", "mp4a.40.2".
    pub codec: String,
    /// Raw codec-specific description bytes (dOps content for Opus, esds content for AAC).
    #[serde(with = "serde_bytes")]
    pub description: Vec<u8>,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of audio channels.
    pub number_of_channels: u32,
    /// Track ID for the MP4 container.
    pub track_id: u32,
    /// Media timescale (ticks per second). Sample durations in fragments are
    /// expressed in this timescale. Matches Hang catalog container.timescale.
    pub timescale: u32,
    /// Optional edit list, preserved from the source for presentation-timeline
    /// fidelity.
    #[serde(skip)]
    pub edits: Option<Vec<EditEntry>>,
}
