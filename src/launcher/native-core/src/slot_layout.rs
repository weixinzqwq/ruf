//! Slot data allocation arithmetic observed at 0x140c511e0.
//! This is a size calculation, not an implementation of SlotStorage or its GC.

pub const SLOT_ALIGNMENT: usize = 8;

/// Eight bytes per payload plus the tag region rounded up to eight bytes.
/// Returns None if the layout exceeds an isize-sized allocation or overflows.
/// The original failure path calls allocation/panic helpers; this recoverable
/// result is a deliberately explicit API choice, not its original ABI.
pub fn allocation_bytes(slot_count: usize) -> Option<usize> {
    let tag_words = (slot_count >> 3) + usize::from(slot_count & 7 != 0);
    let words = slot_count.checked_add(tag_words)?;
    let bytes = words.checked_mul(8)?;
    // Original x64 check: bytes < 0x7ffffffffffffff9.
    // For an 8-byte multiple, this is equivalent to bytes <= isize::MAX - 7.
    (bytes <= (isize::MAX as usize) - 7).then_some(bytes)
}
