//! AOQI reconstruction: compact primitive arrays with a typed GC-safe fallback.
//!
//! Unlike the lost native layout, GC pointers are never encoded as integers.
//! Reference-containing arrays use the upstream Value representation unchanged.

use super::Value;
use gc_arena::Collect;
use std::ops::Range;

const COMPACT_MIN_LEN: usize = 16;

#[derive(Clone, Collect)]
#[collect(no_drop)]
pub enum DenseStorage<'gc> {
    Full(Vec<Option<Value<'gc>>>),
    Primitive { payloads: Vec<u64>, tags: Vec<u8> },
}

fn encode(value: Option<Value<'_>>) -> Option<(u64, u8)> {
    Some(match value {
        None => (0, 255),
        Some(Value::Undefined) => (0, 0),
        Some(Value::Null) => (0, 1),
        Some(Value::Bool(value)) => (u64::from(value), 2),
        Some(Value::Integer(value)) => (u64::from(value as u32), 3),
        Some(Value::Number(value)) => (value.to_bits(), 4),
        Some(Value::String(_) | Value::Object(_)) => return None,
    })
}

fn decode<'gc>(payload: u64, tag: u8) -> Option<Value<'gc>> {
    match tag {
        255 => None,
        0 => Some(Value::Undefined),
        1 => Some(Value::Null),
        2 => Some(Value::Bool(payload != 0)),
        3 => Some(Value::Integer(payload as u32 as i32)),
        4 => Some(Value::Number(f64::from_bits(payload))),
        _ => unreachable!("invalid internal primitive tag"),
    }
}

impl<'gc> DenseStorage<'gc> {
    pub fn with_capacity(capacity: usize) -> Self {
        if capacity >= COMPACT_MIN_LEN {
            Self::Primitive {
                payloads: Vec::with_capacity(capacity),
                tags: Vec::with_capacity(capacity),
            }
        } else {
            Self::Full(Vec::with_capacity(capacity))
        }
    }

    pub fn from_vec(values: Vec<Option<Value<'gc>>>) -> Self {
        if values.len() >= COMPACT_MIN_LEN && values.iter().all(|v| encode(*v).is_some()) {
            let (payloads, tags) = values.into_iter().map(|v| encode(v).unwrap()).unzip();
            Self::Primitive { payloads, tags }
        } else {
            Self::Full(values)
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Full(values) => values.len(),
            Self::Primitive { payloads, .. } => payloads.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<Option<Value<'gc>>> {
        match self {
            Self::Full(values) => values.get(index).copied(),
            Self::Primitive { payloads, tags } => {
                payloads.get(index).map(|p| decode(*p, tags[index]))
            }
        }
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = Option<Value<'gc>>> + ExactSizeIterator {
        (0..self.len()).map(|i| self.get(i).unwrap())
    }

    fn promote(&mut self) {
        if matches!(self, Self::Primitive { .. }) {
            *self = Self::Full(self.iter().collect());
        }
    }

    // Scan a growing full array only once, when it crosses the threshold.
    // Repeated reference inserts must not cause an O(n) scan on every push.
    fn compact_after_growth(&mut self, previous_len: usize) {
        if previous_len < COMPACT_MIN_LEN && self.len() >= COMPACT_MIN_LEN
            && let Self::Full(values) = self
            && values.iter().all(|v| encode(*v).is_some())
        {
            let values = std::mem::take(values);
            *self = Self::from_vec(values);
        }
    }

    pub fn set(&mut self, index: usize, value: Option<Value<'gc>>) {
        assert!(index < self.len());
        let encoded = encode(value);
        if encoded.is_none() { self.promote(); }
        match self {
            Self::Full(values) => values[index] = value,
            Self::Primitive { payloads, tags } => {
                let (payload, tag) = encoded.unwrap();
                payloads[index] = payload;
                tags[index] = tag;
            }
        }
    }

    pub fn resize(&mut self, len: usize) {
        let previous_len = self.len();
        match self {
            Self::Full(values) => values.resize(len, None),
            Self::Primitive { payloads, tags } => {
                payloads.resize(len, 0);
                tags.resize(len, 255);
            }
        }
        self.compact_after_growth(previous_len);
    }

    pub fn push(&mut self, value: Option<Value<'gc>>) {
        self.insert(self.len(), value);
    }

    pub fn pop(&mut self) -> Option<Option<Value<'gc>>> {
        self.len().checked_sub(1).map(|index| self.remove(index))
    }

    pub fn insert(&mut self, index: usize, value: Option<Value<'gc>>) {
        let previous_len = self.len();
        assert!(index <= previous_len);
        let encoded = encode(value);
        if encoded.is_none() { self.promote(); }
        match self {
            Self::Full(values) => values.insert(index, value),
            Self::Primitive { payloads, tags } => {
                let (payload, tag) = encoded.unwrap();
                payloads.insert(index, payload);
                tags.insert(index, tag);
            }
        }
        self.compact_after_growth(previous_len);
    }

    pub fn remove(&mut self, index: usize) -> Option<Value<'gc>> {
        match self {
            Self::Full(values) => values.remove(index),
            Self::Primitive { payloads, tags } => decode(payloads.remove(index), tags.remove(index)),
        }
    }

    pub fn splice(
        &mut self,
        range: Range<usize>,
        items: impl Iterator<Item = Option<Value<'gc>>>,
    ) -> std::vec::IntoIter<Option<Value<'gc>>> {
        let items: Vec<_> = items.collect();
        let previous_len = self.len();
        if items.iter().any(|v| encode(*v).is_none()) { self.promote(); }
        let removed = match self {
            Self::Full(values) => values.splice(range, items).collect::<Vec<_>>(),
            Self::Primitive { payloads, tags } => {
                let encoded: Vec<_> = items.into_iter().map(|v| encode(v).unwrap()).collect();
                let old_payloads = payloads.splice(range.clone(), encoded.iter().map(|v| v.0));
                let old_tags = tags.splice(range, encoded.iter().map(|v| v.1));
                old_payloads.zip(old_tags).map(|(p, t)| decode(p, t)).collect()
            }
        };
        self.compact_after_growth(previous_len);
        removed.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::string::AvmString;
    use gc_arena::{Arena, Rootable};
    use std::mem::size_of;

    fn check(storage: &DenseStorage<'_>, expected: &[Option<Value<'_>>]) {
        assert_eq!(storage.len(), expected.len());
        for (actual, expected) in storage.iter().zip(expected) {
            // Comparing bits retains signed zero and every NaN payload.
            assert_eq!(encode(actual), encode(*expected));
        }
    }

    #[test]
    fn scalar_bit_patterns_holes_and_undefined_are_distinct() {
        let cases = [None, Some(Value::Undefined), Some(Value::Null),
            Some(Value::Bool(false)), Some(Value::Bool(true)),
            Some(Value::Integer(i32::MIN)), Some(Value::Integer(i32::MAX)),
            Some(Value::Number(-0.0)), Some(Value::Number(f64::INFINITY)),
            Some(Value::Number(f64::NEG_INFINITY)),
            Some(Value::Number(f64::from_bits(0x7ff8_1234_5678_9abc))),
            Some(Value::Number(f64::from_bits(1)))];
        let values = cases.repeat(4);
        let storage = DenseStorage::from_vec(values.clone());
        assert!(matches!(storage, DenseStorage::Primitive { .. }));
        check(&storage, &values);
    }

    #[test]
    fn mixed_mutations_match_vec_model() {
        let mut storage = DenseStorage::with_capacity(0);
        let mut model = Vec::new();
        let mut rng = 0x879a_193e_2123u64;
        for step in 0..6000 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = if rng & 7 == 0 { None } else { Some(Value::Number(f64::from_bits(rng))) };
            let index = (rng >> 12) as usize % (model.len() + 1);
            match step % 7 {
                0 => { storage.push(value); model.push(value); }
                1 => { storage.insert(index, value); model.insert(index, value); }
                2 if index < model.len() => {
                    storage.set(index, value); model[index] = value;
                }
                3 => {
                    let size = (rng >> 32) as usize % 128;
                    storage.resize(size); model.resize(size, None);
                }
                4 if index < model.len() => {
                    assert_eq!(encode(storage.remove(index)), encode(model.remove(index)));
                }
                5 => {
                    let end = (index + 3).min(model.len());
                    let old: Vec<_> = storage.splice(index..end, [value, None].into_iter()).collect();
                    let expected: Vec<_> = model.splice(index..end, [value, None]).collect();
                    check(&DenseStorage::from_vec(old), &expected);
                }
                _ => {
                    assert_eq!(storage.pop().map(encode), model.pop().map(encode));
                }
            }
            check(&storage, &model);
        }
    }

    #[test]
    fn growing_arrays_cross_threshold_and_reduce_element_allocation() {
        let mut storage = DenseStorage::with_capacity(0);
        for value in 0..1024 { storage.push(Some(Value::Integer(value))); }
        if let DenseStorage::Primitive { payloads, tags } = storage {
            let allocated = payloads.capacity() * size_of::<u64>() + tags.capacity();
            assert!(allocated < 1024 * size_of::<Option<Value<'_>>>());
            assert_eq!(payloads.len(), tags.len());
        } else { panic!("primitive growth must compact"); }
    }

    #[test]
    fn reference_insertion_promotes_and_survives_collection() {
        let mut arena = Arena::<Rootable![DenseStorage<'_>]>::new(|mc| {
            let mut storage = DenseStorage::from_vec(vec![Some(Value::Null); 32]);
            assert!(matches!(storage, DenseStorage::Primitive { .. }));
            storage.set(8, Some(Value::String(AvmString::new_utf8(mc, "kept by array"))));
            assert!(matches!(storage, DenseStorage::Full(_)));
            storage
        });
        for _ in 0..3 { arena.finish_cycle(); }
        arena.mutate(|_, storage| {
            let Some(Some(Value::String(value))) = storage.get(8) else { panic!("string lost") };
            assert_eq!(value.to_utf8_lossy(), "kept by array");
            assert!(matches!(storage.get(7), Some(Some(Value::Null))));
        });
        arena.mutate_root(|mc, storage| {
            storage.splice(2..4, [Some(Value::String(AvmString::new_utf8(mc, "spliced")))].into_iter()).for_each(drop);
        });
        arena.finish_cycle();
        arena.mutate(|_, storage| {
            let Some(Some(Value::String(value))) = storage.get(2) else { panic!("string lost") };
            assert_eq!(value.to_utf8_lossy(), "spliced");
        });
    }
}
