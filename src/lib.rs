pub mod catalog;
mod error;
mod fragment;
mod init;

// TODO: migrate these modules from old mp4 crate to mp4-atom
// mod sample_table;
// mod timescale;

pub use error::{Error, Result};
pub use fragment::{fragment_to_directory, fragment_track, FragmentStats, TrackStats};
pub use init::{catalog_from_mp4, build_init_segment, read_moov};
