//! Circuit-level building blocks.
//!
//! Everything a pedal in this project is made of lives here: the RC sections
//! that shape a signal, the diode equation that clips it, the oversampling that
//! keeps the clipping honest, the delay lines and companders that make a
//! bucket-brigade device, and the detectors that drive a compressor.
//!
//! The rule for this module is that a value should be traceable to something
//! measurable — a component value, a datasheet number, a corner frequency —
//! rather than tuned until it sounded acceptable.

pub mod analog;
pub mod delay;
pub mod dynamics;
pub mod filters;
pub mod nonlinear;
pub mod opamp;
pub mod ota;
pub mod oversample;
pub mod rectifier;
pub mod source;
pub mod spring;
pub mod tonestack;
pub mod transistor;
