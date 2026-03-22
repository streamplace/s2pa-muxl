//! CBOR (DRISL) serialization for MUXL streaming events.
//!
//! Defines the wire format for the `--stdout` streaming protocol.
//! Each event is a separate CBOR value in the stream.
//!
//! ```cbor
//! {"type": "init", "data": h'<ftyp+moov bytes>'}
//! {"type": "segment", "tracks": {"1": h'<video moof+mdat>', "2": h'<audio moof+mdat>'}}
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    },
    /// A complete GOP segment with all tracks bundled.
    ///
    /// Track keys are stringified track IDs (DRISL requires string map keys).
    /// Values are CBOR byte strings containing per-track moof+mdat data.
    #[serde(rename = "segment")]
    Segment {
        tracks: BTreeMap<String, ByteString>,
    },
}

impl CborEvent {
    /// Convert a [`SegmenterEvent`] reference into a serializable CBOR event.
    pub fn from_event(event: &SegmenterEvent) -> Self {
        match event {
            SegmenterEvent::InitSegment { data, .. } => CborEvent::Init { data: data.clone() },
            SegmenterEvent::Segment(gop) => CborEvent::Segment {
                tracks: gop
                    .tracks
                    .iter()
                    .map(|(tid, data)| (tid.to_string(), ByteString(data.clone())))
                    .collect(),
            },
        }
    }

    /// Convert a [`SegmenterEvent`] into a serializable CBOR event (owned).
    pub fn from_event_owned(event: SegmenterEvent) -> Self {
        match event {
            SegmenterEvent::InitSegment { data, .. } => CborEvent::Init { data },
            SegmenterEvent::Segment(gop) => CborEvent::Segment {
                tracks: gop
                    .tracks
                    .into_iter()
                    .map(|(tid, data)| (tid.to_string(), ByteString(data)))
                    .collect(),
            },
        }
    }
}
