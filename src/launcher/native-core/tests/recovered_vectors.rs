use aoqi_recovered_core::dense_storage::{encode, set_encoded, DenseBlock, EncodedValue};
use aoqi_recovered_core::{runtime_config, slot_layout};

// Expected tags/payloads are transcribed from the original PE jump-table cases,
// not calculated by calling another version of the implementation.
#[test]
fn all_scalar_cases_and_object_endpoints_match_machine_code() {
    let payload = 0xfedc_ba98_7654_32a5;
    let vectors = [
        (u64::MAX, 255, 0),
        (0x34, 0, 0),
        (0x35, 1, 0),
        (0x36, 2, 0xa5),
        (0x37, 4, 0xfedc_ba98_7654_32a5),
        (0x38, 3, 0x7654_32a5),
        (0x39, 5, 0xfedc_ba98_7654_32a5),
        (0, 6, 0xfedc_ba98_7654_32a5),
        (0x33, 57, 0xfedc_ba98_7654_32a5),
    ];
    for (discriminant, tag, expected_payload) in vectors {
        assert_eq!(encode(discriminant, payload), Some(EncodedValue { payload: expected_payload, tag }));
    }
    assert_eq!(encode(0x3a, payload), None);
    assert_eq!(encode(u64::MAX - 1, payload), None);
}

#[test]
fn every_object_discriminant_is_preserved_as_its_own_tag() {
    let observed_tags = (0..=0x33).map(|d| encode(d, 17).unwrap().tag).collect::<Vec<_>>();
    assert_eq!(observed_tags, (6u8..=57).collect::<Vec<_>>());
}

#[test]
fn block_abi_matches_payload_and_tag_offsets() {
    assert_eq!(std::mem::size_of::<DenseBlock>(), 72);
    assert_eq!(std::mem::align_of::<DenseBlock>(), 8);
    assert_eq!(std::mem::offset_of!(DenseBlock, payloads), 0);
    assert_eq!(std::mem::offset_of!(DenseBlock, tags), 64);
}

#[test]
fn writes_cross_block_boundary_without_overwriting_neighbors() {
    let mut blocks = [DenseBlock::sentinel_filled(); 2];
    set_encoded(&mut blocks, 9, 7, encode(0x36, 0x1a5).unwrap());
    set_encoded(&mut blocks, 9, 8, encode(0x38, 0xffff_ffff_1234_5678).unwrap());
    assert_eq!(blocks[0].payloads, [0, 0, 0, 0, 0, 0, 0, 0xa5]);
    assert_eq!(blocks[0].tags, [255, 255, 255, 255, 255, 255, 255, 2]);
    assert_eq!(blocks[1].payloads, [0x1234_5678, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(blocks[1].tags, [3, 255, 255, 255, 255, 255, 255, 255]);
}

#[test]
#[should_panic(expected = "index must be below logical length")]
fn unused_capacity_does_not_make_an_index_valid() {
    let mut blocks = [DenseBlock::sentinel_filled(); 2];
    set_encoded(&mut blocks, 9, 9, encode(0x34, 0).unwrap());
}

#[test]
fn slot_sizes_cover_partial_groups_and_allocation_failure() {
    for (slots, bytes) in [(0, 0), (1, 16), (7, 64), (8, 72), (9, 88), (15, 136), (16, 144)] {
        assert_eq!(slot_layout::allocation_bytes(slots), Some(bytes));
    }
    assert_eq!(slot_layout::SLOT_ALIGNMENT, 8);
    assert_eq!(slot_layout::allocation_bytes(usize::MAX), None);
    assert_eq!(slot_layout::allocation_bytes(isize::MAX as usize), None);
}

#[test]
fn render_scale_preserves_only_the_observed_integer_range() {
    assert_eq!(runtime_config::render_scale(None), 1);
    for (value, expected) in [(0, 1), (1, 1), (2, 2), (3, 3), (4, 4), (5, 1), (u64::MAX, 1)] {
        assert_eq!(runtime_config::render_scale(Some(value)), expected);
    }
}

#[test]
fn slice_rejects_zero_negative_and_nonfinite_before_clamping() {
    assert_eq!(runtime_config::gc_slice_ms(None), 4.0);
    for invalid in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(runtime_config::gc_slice_ms(Some(invalid)), 4.0);
    }
    for (input, expected) in [(f64::from_bits(1), 0.1), (0.05, 0.1), (0.1, 0.1), (4.0, 4.0), (12.0, 12.0), (13.0, 12.0), (f64::MAX, 12.0)] {
        assert_eq!(runtime_config::gc_slice_ms(Some(input)), expected);
    }
}

#[test]
fn sleep_accepts_both_zeros_and_has_no_upper_clamp() {
    assert_eq!(runtime_config::gc_sleep_factor(None), 0.1);
    for invalid in [-1.0, -f64::from_bits(1), f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(runtime_config::gc_sleep_factor(Some(invalid)), 0.1);
    }
    for input in [0.0, -0.0, f64::from_bits(1), 0.1, 12.0, f64::MAX] {
        assert_eq!(runtime_config::gc_sleep_factor(Some(input)).to_bits(), input.to_bits());
    }
}

// Independent bit predicate transcribed from x64 instructions:
// 140ba7a1d..140ba7a66 and 140ba7c6d..140ba7cbf.
fn original_accepts(bits: u64, accept_zero: bool) -> bool {
    let magnitude = bits & 0x7fff_ffff_ffff_ffff;
    let positive_normal = (bits as i64) >= 0
        && (magnitude.wrapping_add(0xfff0_0000_0000_0000) >> 53) < 0x3ff;
    let positive_subnormal = bits.wrapping_sub(1) < 0x000f_ffff_ffff_ffff;
    positive_normal || positive_subnormal || (accept_zero && magnitude == 0)
}

#[test]
fn gc_selection_matches_native_bit_predicate_across_all_exponents() {
    for sign in [0u64, 1u64 << 63] {
        for exponent in 0u64..=0x7ff {
            for mantissa in [0, 1, 0x0007_ffff_ffff_ffff, 0x000f_ffff_ffff_ffff] {
                let bits = sign | (exponent << 52) | mantissa;
                let value = f64::from_bits(bits);
                let slice = if original_accepts(bits, false) {
                    if value < 0.1 { 0.1 } else if value > 12.0 { 12.0 } else { value }
                } else {
                    4.0
                };
                let sleep = if original_accepts(bits, true) { value } else { 0.1 };
                assert_eq!(runtime_config::gc_slice_ms(Some(value)).to_bits(), slice.to_bits(), "slice bits={bits:x}");
                assert_eq!(runtime_config::gc_sleep_factor(Some(value)).to_bits(), sleep.to_bits(), "sleep bits={bits:x}");
            }
        }
    }
}
