//! Valid-value encoding observed at 0x140ce83c0 and 0x140ce8660.
//! Discriminants are build-specific numbers, not recovered Rust variant names.
//! This module does not implement allocation, GC tracing, or representation conversion.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodedValue {
    pub payload: u64,
    pub tag: u8,
}

/// Encode a valid raw `(discriminant, payload)` pair from the selected binary.
/// `None` is this reconstruction's boundary check for unsupported discriminants;
/// the original compiled enum's behavior on invalid representations is unknown.
pub fn encode(discriminant: u64, payload: u64) -> Option<EncodedValue> {
    let (tag, payload) = match discriminant {
        u64::MAX => (255, 0),
        0x34 => (0, 0),
        0x35 => (1, 0),
        0x36 => (2, payload & 0xff),
        0x37 => (4, payload),
        0x38 => (3, payload & 0xffff_ffff),
        0x39 => (5, payload),
        0..=0x33 => ((discriminant + 6) as u8, payload),
        _ => return None,
    };
    Some(EncodedValue { payload, tag })
}

/// The observed 72-byte block: eight payload words, then eight tag bytes.
/// Payload words have the host's endianness; the original target is little endian.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DenseBlock {
    pub payloads: [u64; 8],
    pub tags: [u8; 8],
}

impl DenseBlock {
    /// Fill with the observed UINT64_MAX sentinel encoding.
    /// This helper does not claim that all original allocations used this initializer.
    pub const fn sentinel_filled() -> Self {
        Self {
            payloads: [0; 8],
            tags: [255; 8],
        }
    }
}

/// Write an already encoded value using the original block/lane addressing.
/// Storage allocation and logical length are supplied by the caller.
pub fn set_encoded(
    blocks: &mut [DenseBlock],
    logical_len: usize,
    index: usize,
    value: EncodedValue,
) {
    assert!(index < logical_len, "index must be below logical length");
    let block = &mut blocks[index >> 3];
    let lane = index & 7;
    block.payloads[lane] = value.payload;
    block.tags[lane] = value.tag;
}
