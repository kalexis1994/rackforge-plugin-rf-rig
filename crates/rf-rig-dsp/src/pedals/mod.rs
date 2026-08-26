//! The pedals themselves.
//!
//! Each module models one circuit family and owns its own component values.
//! None of them know about the chain, the host, or each other: a pedal takes a
//! sample (and, where it needs memory, a slice) and returns a sample. The order
//! they run in belongs to [`crate::Engine`].

pub mod chorus;
pub mod compressor;
pub mod distortion;
pub mod echo;
pub mod fuzz;
pub mod overdrive;
pub mod reverb;
