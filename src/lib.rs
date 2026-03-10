pub mod catalog;
mod error;
mod fragment;
mod init;
mod segment;

pub use error::{Error, Result};
pub use fragment::{fragment_fmp4, fragment_to_directory, fragment_track, FMP4Reader, Frame, FragmentStats, TrackStats};
pub use init::{catalog_from_moov, catalog_from_mp4, build_init_segment, read_moov};
pub use segment::{segment_fmp4, Segment};
