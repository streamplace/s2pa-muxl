//! WebAssembly bindings for the MUXL segmenter.
//!
//! Exposes a `WasmSegmenter` class to JavaScript via wasm-bindgen.
//!
//! ```js
//! import init, { WasmSegmenter } from './muxl.js';
//! await init();
//!
//! const segmenter = new WasmSegmenter();
//! const response = await fetch('stream.mp4');
//! const reader = response.body.getReader();
//!
//! while (true) {
//!   const { done, value } = await reader.read();
//!   if (done) break;
//!   const events = segmenter.feed(value);
//!   for (const event of events) {
//!     if (event.type === 'init') {
//!       // event.data is a Uint8Array with the canonical init segment
//!     } else if (event.type === 'segment') {
//!       // event.number is the segment number
//!       // event.data is a Uint8Array with the segment bytes
//!     }
//!   }
//! }
//! // Flush any remaining partial segment
//! const final_events = segmenter.flush();
//! ```

use js_sys::{Array, Object, Reflect, SharedArrayBuffer, Uint8Array};
use wasm_bindgen::prelude::*;

use crate::push::{Segmenter, SegmenterEvent};
use crate::wasm_io::{WasmReadAt, WasmWriteAt};

/// MUXL streaming segmenter for WebAssembly.
///
/// Feed fMP4 chunks via `feed()`, receive init segments and MUXL segments
/// as JavaScript objects.
#[wasm_bindgen]
pub struct WasmSegmenter {
    inner: Segmenter,
}

#[wasm_bindgen]
impl WasmSegmenter {
    /// Create a new segmenter ready to receive fMP4 data.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        WasmSegmenter {
            inner: Segmenter::new(),
        }
    }

    /// Feed a chunk of fMP4 data. Returns an array of event objects.
    ///
    /// Each event is `{ type: "init", data: Uint8Array }` or
    /// `{ type: "segment", number: number, data: Uint8Array }`.
    pub fn feed(&mut self, data: &[u8]) -> Result<Array, JsValue> {
        let events = self
            .inner
            .feed(data)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(events_to_js(events))
    }

    /// Signal end of stream. Returns any remaining partial segment.
    pub fn flush(&mut self) -> Result<Array, JsValue> {
        let events = self
            .inner
            .flush()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(events_to_js(events))
    }
}

/// Convert a flat MP4 to a MUXL archive, streaming I/O through SharedArrayBuffers.
///
/// - `read_sab`: SharedArrayBuffer for reading from the input file (ReadAt protocol)
/// - `write_sab`: SharedArrayBuffer for writing archive output (Write protocol)
///
/// The main thread must:
/// 1. Fulfill read requests on `read_sab` (via `Blob.slice()`)
/// 2. Drain write chunks from `write_sab` (to BLAKE3 hasher + S3 upload)
///
/// Returns a JSON string containing the track metadata (codecs, segments,
/// init CIDs, byte offsets). Init segment data is written to `write_sab`
/// before the archive data, prefixed by a 4-byte LE length for each track's
/// init segment.
#[wasm_bindgen]
pub fn convert_flat_mp4(read_sab: &SharedArrayBuffer, write_sab: &SharedArrayBuffer) -> Result<String, JsValue> {
    let reader = WasmReadAt::new(read_sab)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    let mut writer = WasmWriteAt::new(write_sab)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let tracks = crate::flat_mp4_to_archive(&reader, &mut writer)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    // Write init segments to the output stream so the main thread can upload them.
    // Format: [4-byte LE length][init data] for each track, in order.
    use std::io::Write;
    for track in &tracks {
        let len = (track.init_data.len() as u32).to_le_bytes();
        writer.write_all(&len).map_err(|e| JsValue::from_str(&e.to_string()))?;
        writer.write_all(&track.init_data).map_err(|e| JsValue::from_str(&e.to_string()))?;
    }

    // Signal end of stream
    writer.finish();

    // Return track metadata as JSON (small — just offsets, codecs, CIDs)
    let json = serde_json::to_string(&tracks)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(json)
}

fn events_to_js(events: Vec<SegmenterEvent>) -> Array {
    let arr = Array::new();
    for event in events {
        let obj = Object::new();
        match event {
            SegmenterEvent::InitSegment { data, .. } => {
                Reflect::set(&obj, &"type".into(), &"init".into()).unwrap();
                let buf = Uint8Array::from(data.as_slice());
                Reflect::set(&obj, &"data".into(), &buf).unwrap();
            }
            SegmenterEvent::Segment(seg) => {
                Reflect::set(&obj, &"type".into(), &"segment".into()).unwrap();
                Reflect::set(&obj, &"number".into(), &JsValue::from(seg.number)).unwrap();
                let buf = Uint8Array::from(seg.data.as_slice());
                Reflect::set(&obj, &"data".into(), &buf).unwrap();
            }
        }
        arr.push(&obj);
    }
    arr
}
