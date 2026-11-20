//! Reconstruction settings for the recovered bitmap shader and idle GPU cache.
//! These APIs are new; they do not claim the original host's configuration ABI.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BitmapUpscale {
    /// Preserve the upstream sampler and color transform.
    #[default]
    Off,
    /// Preserve exact texels for small, axis-aligned integer enlargement.
    PixelArt,
    /// Use the recovered EASU filter, with the same small-bitmap protection.
    Easu,
}

impl BitmapUpscale {
    pub fn from_setting(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("1" | "pixel" | "pixel-art") => Self::PixelArt,
            Some("2" | "easu") => Self::Easu,
            _ => Self::Off,
        }
    }

    pub(crate) fn uniform(self) -> [f32; 4] {
        let value = match self {
            Self::Off => 0.0,
            Self::PixelArt => 1.0,
            Self::Easu => 2.0,
        };
        [value, 0.0, 0.0, 0.0]
    }
}

/// Limits apply only to reusable textures, never textures still held by a draw.
/// DX12 textures use native allocation requirements; other backends use descriptor estimates.
/// Neither value measures driver residency or in-flight GPU retention.
#[derive(Clone, Copy, Debug)]
pub struct TexturePoolLimits {
    pub idle_bytes: u64,
    pub idle_frames: u64,
}

impl Default for TexturePoolLimits {
    fn default() -> Self {
        Self {
            idle_bytes: 256 * 1024 * 1024,
            idle_frames: 120,
        }
    }
}

pub(crate) fn byte_budget(setting: Option<&str>, default_megabytes: u64) -> u64 {
    setting
        .and_then(|s| s.trim().parse::<u64>().ok())
        .and_then(|n| n.checked_mul(1024 * 1024))
        .unwrap_or(default_megabytes * 1024 * 1024)
}

impl TexturePoolLimits {
    pub fn from_settings(megabytes: Option<&str>, frames: Option<&str>) -> Self {
        let defaults = Self::default();
        Self {
            idle_bytes: megabytes
                .and_then(|s| s.trim().parse::<u64>().ok())
                .and_then(|n| n.checked_mul(1024 * 1024))
                .unwrap_or(defaults.idle_bytes),
            idle_frames: frames
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(defaults.idle_frames),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_modes_have_the_recovered_numeric_contract() {
        assert_eq!(BitmapUpscale::default().uniform(), [0.0; 4]);
        assert_eq!(BitmapUpscale::PixelArt.uniform(), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(BitmapUpscale::Easu.uniform(), [2.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn settings_have_finite_defaults_and_zero_disables_idle_retention() {
        assert_eq!(byte_budget(None, 64), 64 * 1024 * 1024);
        assert_eq!(byte_budget(Some("0"), 384), 0);
        assert_eq!(byte_budget(Some("18446744073709551615"), 384), 384 * 1024 * 1024);
        assert_eq!(BitmapUpscale::from_setting(Some("easu")), BitmapUpscale::Easu);
        assert_eq!(BitmapUpscale::from_setting(Some("1")), BitmapUpscale::PixelArt);
        assert_eq!(BitmapUpscale::from_setting(Some("invalid")), BitmapUpscale::Off);
        let limits = TexturePoolLimits::from_settings(Some("0"), Some("0"));
        assert_eq!(limits.idle_bytes, 0);
        assert_eq!(limits.idle_frames, 0);
        let limits = TexturePoolLimits::from_settings(Some("18446744073709551615"), Some("bad"));
        assert_eq!(limits.idle_bytes, TexturePoolLimits::default().idle_bytes);
        assert_eq!(limits.idle_frames, 120);
    }
}
