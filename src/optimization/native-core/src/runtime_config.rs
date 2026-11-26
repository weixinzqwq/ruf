//! Numeric selection reconstructed from native instructions and PE constants.
//! These functions accept parsed values; environment and JSON parsers are not recovered.

/// 0x1404c7770 accepts the unsigned-integer number representation and values 1..4.
/// Callers pass None for a missing field or a different JSON number/value kind.
pub fn render_scale(unsigned_integer: Option<u64>) -> u32 {
    match unsigned_integer {
        Some(value @ 1..=4) => value as u32,
        _ => 1,
    }
}

/// 0x140ba78e0: positive finite values are clamped to 0.1..12.0 milliseconds.
/// Missing, failed parse, zero, negative, and non-finite values use 4.0.
pub fn gc_slice_ms(parsed: Option<f64>) -> f64 {
    match parsed {
        Some(value) if value.is_finite() && value > 0.0 => value.clamp(0.1, 12.0),
        _ => 4.0,
    }
}

/// 0x140ba7b30: nonnegative finite values are accepted without an upper clamp.
/// Both signed zeros are accepted; missing or invalid values use 0.1.
pub fn gc_sleep_factor(parsed: Option<f64>) -> f64 {
    match parsed {
        Some(value) if value.is_finite() && value >= 0.0 => value,
        _ => 0.1,
    }
}
