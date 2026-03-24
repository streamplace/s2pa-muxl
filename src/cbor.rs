//! CBOR (DRISL) serialization for MUXL streaming events.
//!
//! Defines the wire format for the `--stdout` streaming protocol.
//! Each event is a separate CBOR value in the stream.
//!
//! ```cbor
//! {"type": "init", "data": h'<ftyp+moov bytes>', "catalog": {"video": {...}, "audio": {...}}}
//! {"type": "segment", "tracks": {"1": h'<video moof+mdat>', "2": h'<audio moof+mdat>'},
//!  "durations": {"1": 60000, "2": 48000}, "sample_counts": {"1": 60, "2": 50}}
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::init::build_track_init_segments;
use crate::push::SegmenterEvent;

/// Wrapper for `Vec<u8>` that serializes as a CBOR byte string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ByteString(#[serde(with = "serde_bytes")] pub Vec<u8>);

/// A MUXL streaming event in CBOR-serializable form.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum CborEvent {
    /// Canonical init segment (ftyp+moov).
    #[serde(rename = "init")]
    Init {
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
        /// Track configuration metadata (codecs, dimensions, timescales, etc.).
        #[serde(default)]
        catalog: Option<Catalog>,
        /// Per-track init segments (single-track ftyp+moov), keyed by stringified track ID.
        /// Used by HLS CMAF media playlists where each track needs its own init segment.
        #[serde(default)]
        track_inits: BTreeMap<String, ByteString>,
    },
    /// A complete GOP segment with all tracks bundled.
    ///
    /// Track keys are stringified track IDs (DRISL requires string map keys).
    /// Values are CBOR byte strings containing per-track moof+mdat data.
    #[serde(rename = "segment")]
    Segment {
        tracks: BTreeMap<String, ByteString>,
        /// Per-track total duration in timescale ticks.
        #[serde(default)]
        durations: BTreeMap<String, u64>,
        /// Per-track sample (frame) count.
        #[serde(default)]
        sample_counts: BTreeMap<String, u32>,
    },
}

impl CborEvent {
    /// Convert a [`SegmenterEvent`] reference into a serializable CBOR event.
    pub fn from_event(event: &SegmenterEvent) -> Self {
        match event {
            SegmenterEvent::InitSegment { catalog, data } => {
                let track_inits = build_track_init_segments(catalog)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(tid, bytes)| (tid.to_string(), ByteString(bytes)))
                    .collect();
                CborEvent::Init {
                    data: data.clone(),
                    catalog: Some(catalog.clone()),
                    track_inits,
                }
            }
            SegmenterEvent::Segment(gop) => CborEvent::Segment {
                tracks: gop
                    .tracks
                    .iter()
                    .map(|(tid, data)| (tid.to_string(), ByteString(data.clone())))
                    .collect(),
                durations: gop
                    .durations
                    .iter()
                    .map(|(tid, dur)| (tid.to_string(), *dur))
                    .collect(),
                sample_counts: gop
                    .sample_counts
                    .iter()
                    .map(|(tid, count)| (tid.to_string(), *count))
                    .collect(),
            },
        }
    }

    /// Convert a [`SegmenterEvent`] into a serializable CBOR event (owned).
    pub fn from_event_owned(event: SegmenterEvent) -> Self {
        match event {
            SegmenterEvent::InitSegment { catalog, data } => {
                let track_inits = build_track_init_segments(&catalog)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(tid, bytes)| (tid.to_string(), ByteString(bytes)))
                    .collect();
                CborEvent::Init {
                    data,
                    catalog: Some(catalog),
                    track_inits,
                }
            }
            SegmenterEvent::Segment(gop) => CborEvent::Segment {
                tracks: gop
                    .tracks
                    .into_iter()
                    .map(|(tid, data)| (tid.to_string(), ByteString(data)))
                    .collect(),
                durations: gop
                    .durations
                    .into_iter()
                    .map(|(tid, dur)| (tid.to_string(), dur))
                    .collect(),
                sample_counts: gop
                    .sample_counts
                    .into_iter()
                    .map(|(tid, count)| (tid.to_string(), count))
                    .collect(),
            },
        }
    }
}
