//! Float maths routed through `libm`.
//!
//! The plugin is `no_std` on wasm, where the `f32` inherent methods do not
//! exist. Routing every call through one module keeps native tests and the
//! packaged component running the *same* arithmetic, which is the only way a
//! measurement taken on the desktop says anything about the wasm build.

pub const TAU: f32 = core::f32::consts::TAU;
pub const PI: f32 = core::f32::consts::PI;

#[inline]
pub fn exp(value: f32) -> f32 {
    libm::expf(value)
}

#[inline]
pub fn ln(value: f32) -> f32 {
    libm::logf(value)
}

#[inline]
pub fn sin(value: f32) -> f32 {
    libm::sinf(value)
}

#[inline]
pub fn cos(value: f32) -> f32 {
    libm::cosf(value)
}

#[inline]
pub fn tan(value: f32) -> f32 {
    libm::tanf(value)
}

#[inline]
pub fn tanh(value: f32) -> f32 {
    libm::tanhf(value)
}

#[inline]
pub fn sinh(value: f32) -> f32 {
    libm::sinhf(value)
}

#[inline]
pub fn cosh(value: f32) -> f32 {
    libm::coshf(value)
}

#[inline]
pub fn sqrt(value: f32) -> f32 {
    libm::sqrtf(value)
}

/// Double-precision square root, for the places that solve a network before
/// the coefficients are normalised.
#[inline]
pub fn sqrt64(value: f64) -> f64 {
    libm::sqrt(value)
}

#[inline]
pub fn abs(value: f32) -> f32 {
    libm::fabsf(value)
}

#[inline]
pub fn floor(value: f32) -> f32 {
    libm::floorf(value)
}

#[inline]
pub fn powf(value: f32, exponent: f32) -> f32 {
    libm::powf(value, exponent)
}

/// Decibels to linear amplitude.
#[inline]
pub fn db_to_gain(decibels: f32) -> f32 {
    exp(decibels * (core::f32::consts::LN_10 / 20.0))
}

#[inline]
pub fn clamp(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

/// Linear interpolation.
#[inline]
pub fn lerp(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

/// Maps a normalised 0..1 knob onto an exponential range, which is how a real
/// audio-taper pot behaves and how every time or frequency control here is
/// scaled.
#[inline]
pub fn exponential(normalised: f32, minimum: f32, maximum: f32) -> f32 {
    minimum * powf(maximum / minimum, clamp(normalised, 0.0, 1.0))
}

/// Replaces a denormal or non-finite sample with silence. Feedback paths run
/// for hours at a time on a stage; one NaN would stay forever.
#[inline]
pub fn sanitise(value: f32) -> f32 {
    if value.is_finite() && abs(value) > 1.0e-20 {
        value
    } else {
        0.0
    }
}
