//! Object representation for `flash.utils.Dictionary`

use crate::avm2::{Error, WeakDictionaryStats};
use crate::avm2::activation::Activation;
use crate::avm2::dynamic_map::{DynamicKey, DynamicMap};
use crate::avm2::object::script_object::ScriptObjectData;
use crate::avm2::object::{ClassObject, Object, StageObject, StageObjectWeak, TObject, WeakObject};
use crate::avm2::value::Value;
use crate::string::AvmString;
use core::fmt;
use gc_arena::{Collect, Gc, GcWeak, Mutation};
use gc_arena::barrier::unlock;
use gc_arena::collect::Trace;
use gc_arena::lock::{Lock, RefLock};
use ruffle_common::utils::HasPrefixField;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A class instance allocator that allocates Dictionary objects.
pub fn dictionary_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    let base = ScriptObjectData::new(class);

    Ok(DictionaryObject(Gc::new(activation.gc(), DictionaryObjectData {
        base,
        weak_keys: Lock::new(None),
        display_owners: Cell::new(false),
        owner_keys: RefLock::new(Vec::new()),
    })).into())
}

/// An object that allows associations between objects and values.
///
/// This is implemented by way of "object space", parallel to the property
/// space that ordinary properties live in. This space has no namespaces, and
/// keys are objects instead of strings.
#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub struct DictionaryObject<'gc>(pub Gc<'gc, DictionaryObjectData<'gc>>);

#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct DictionaryObjectWeak<'gc>(pub GcWeak<'gc, DictionaryObjectData<'gc>>);

impl fmt::Debug for DictionaryObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DictionaryObject")
            .field("ptr", &Gc::as_ptr(self.0))
            .finish()
    }
}

#[derive(Clone, Collect, HasPrefixField)]
#[collect(no_drop)]
#[repr(C, align(8))]
pub struct DictionaryObjectData<'gc> {
    /// Base script object
    base: ScriptObjectData<'gc>,
    weak_keys: Lock<Option<Gc<'gc, WeakKeyTable<'gc>>>>,
    #[collect(require_static)]
    display_owners: Cell<bool>,
    owner_keys: RefLock<Vec<Option<StageObjectWeak<'gc>>>>,
}

struct WeakKeyTable<'gc> {
    entries: RefCell<DynamicMap<WeakObject<'gc>, WeakDictionaryValue<'gc>>>,
    stats: Rc<Cell<WeakDictionaryStats>>,
}

#[derive(Clone, Copy)]
enum WeakDictionaryValue<'gc> {
    // A dictionary value can lead back to its key, including via other dictionaries.
    // Keep only a weak handle here. The collector retains it when the key is marked.
    Object(WeakObject<'gc>),
    Primitive(Value<'gc>),
}

impl<'gc> WeakDictionaryValue<'gc> {
    fn new(value: Value<'gc>) -> Self {
        match value {
            Value::Object(object) => Self::Object(object.downgrade()),
            value => Self::Primitive(value),
        }
    }

    fn get(self, mc: &Mutation<'gc>) -> Value<'gc> {
        match self {
            Self::Object(object) => object.upgrade(mc).map_or(Value::Undefined, Value::from),
            Self::Primitive(value) => value,
        }
    }
}

// SAFETY: Only removal happens here. For object values both weak pointers
// are registered as an ephemeron. For primitives all remaining pointers are
// traced normally. Writes that adopt pointers use the barrier on the table.
unsafe impl<'gc> Collect<'gc> for WeakKeyTable<'gc> {
    fn trace<C: Trace<'gc>>(&self, cc: &mut C) {
        let mut entries = self.entries.borrow_mut();
        let previous = entries.len();
        entries.retain(|key, value| {
            if key.is_dropped() {
                return false;
            }
            match *value {
                WeakDictionaryValue::Object(value) => cc.trace_ephemeron(key.erase(), value.erase()),
                WeakDictionaryValue::Primitive(value) => {
                    cc.trace(key);
                    cc.trace(&value);
                }
            }
            true
        });
        let removed = previous - entries.len();
        if removed != 0 {
            if entries.is_empty() {
                *entries = DynamicMap::new();
            }
            let mut stats = self.stats.get();
            stats.pruned += removed as u64;
            self.stats.set(stats);
        }
    }
}

// AVM2 enumeration indices are positive i32. Keep separate index spaces
// for ordinary properties, general weak keys and the display-owner extension.
const WEAK: u32 = 0x2000_0000;
const OWNER: u32 = 0x4000_0000;

impl<'gc> DictionaryObject<'gc> {
    pub fn has_weak_keys(self) -> bool {
        self.0.weak_keys.get().is_some()
    }

    pub fn enable_weak_keys(self, activation: &mut Activation<'_, 'gc>) {
        if self.has_weak_keys() {
            return;
        }
        let mc = activation.gc();
        let table = Gc::new(mc, WeakKeyTable {
            entries: RefCell::new(DynamicMap::new()),
            stats: activation.avm2().weak_dictionary_stats.clone(),
        });
        unlock!(Gc::write(mc, self.0), DictionaryObjectData, weak_keys).set(Some(table));
        // A subclass may have populated the dictionary before invoking super.
        let objects: Vec<_> = self.base().values().keys()
            .filter_map(|key| match key {
                DynamicKey::Object(key) => Some(*key),
                _ => None,
            })
            .collect();
        for key in objects {
            let entry = self.base().values_mut(mc).remove(&DynamicKey::Object(key)).unwrap();
            self.set_property_by_object(key, entry.value, mc);
        }
    }

    fn owner_id(self) -> GcWeak<'gc, ()> {
        GcWeak::erase(Gc::downgrade(self.0))
    }

    /// Explicit opt-in extension used by the recovered FairyGUI bridge.
    /// Non-display keys retain the dictionary's selected strong or weak semantics.
    pub fn use_display_owners(self, mc: &Mutation<'gc>) {
        if self.0.display_owners.replace(true) { return; }
        if let Some(table) = self.0.weak_keys.get() {
            let displays: Vec<_> = table.entries.borrow().keys()
                .filter_map(|key| match key {
                    WeakObject::StageObject(display) => display.0.upgrade(mc).map(StageObject),
                    _ => None,
                })
                .collect();
            for display in displays {
                let key = Object::from(display);
                let entry = table.entries.borrow_mut().remove(&key.downgrade()).unwrap();
                self.set_property_by_object(key, entry.value.get(mc), mc);
            }
        }
        let entries: Vec<_> = self.base().values().iter().filter_map(|(key, entry)| {
            if let DynamicKey::Object(object) = key {
                object.as_stage_object().map(|owner| (owner, entry.value))
            } else { None }
        }).collect();
        for (owner, value) in entries {
            self.base().values_mut(mc).remove(&DynamicKey::Object(owner.into()));
            self.set_property_by_object(owner.into(), value, mc);
        }
    }

    fn owner(self, name: Object<'gc>) -> Option<StageObject<'gc>> {
        self.0.display_owners.get().then(|| name.as_stage_object()).flatten()
    }

    /// Retrieve a value in the dictionary's object space.
    pub fn get_property_by_object(self, name: Object<'gc>, mc: &Mutation<'gc>) -> Value<'gc> {
        if let Some(owner) = self.owner(name) {
            return owner.display_owner_value(self.owner_id()).unwrap_or(Value::Undefined);
        }
        if let Some(table) = self.0.weak_keys.get() {
            return table.entries.borrow().get(&name.downgrade())
                .map_or(Value::Undefined, |entry| entry.value.get(mc));
        }
        self.base()
            .values()
            .get(&DynamicKey::Object(name))
            .map(|v| v.value)
            .unwrap_or(Value::Undefined)
    }

    /// Set a value in the dictionary's object space.
    pub fn set_property_by_object(self, name: Object<'gc>, value: Value<'gc>, mc: &Mutation<'gc>) {
        if let Some(owner) = self.owner(name) {
            owner.set_display_owner_value(self.owner_id(), value, mc);
            let mut keys = unlock!(Gc::write(mc, self.0), DictionaryObjectData, owner_keys).borrow_mut();
            if !keys.iter().flatten().any(|key| GcWeak::as_ptr(key.0) == Gc::as_ptr(owner.0)) {
                let key = Some(StageObjectWeak(Gc::downgrade(owner.0)));
                if let Some(slot) = keys.iter_mut().find(|slot| slot.is_none_or(|key| key.0.is_dropped())) {
                    *slot = key;
                } else { keys.push(key); }
            }
            return;
        }
        if let Some(table) = self.0.weak_keys.get() {
            let mut entries = Gc::write(mc, table).entries.borrow_mut();
            let previous = entries.len();
            entries.insert(name.downgrade(), WeakDictionaryValue::new(value));
            if entries.len() > previous {
                let mut stats = table.stats.get();
                stats.inserted += 1;
                table.stats.set(stats);
            }
            return;
        }
        self.base()
            .values_mut(mc)
            .insert(DynamicKey::Object(name), value);
    }

    /// Delete a value from the dictionary's object space.
    pub fn delete_property_by_object(self, name: Object<'gc>, mc: &Mutation<'gc>) {
        if let Some(owner) = self.owner(name) {
            owner.remove_display_owner_value(self.owner_id(), mc);
            for key in unlock!(Gc::write(mc, self.0), DictionaryObjectData, owner_keys).borrow_mut().iter_mut() {
                if key.is_some_and(|key| GcWeak::as_ptr(key.0) == Gc::as_ptr(owner.0)) { *key = None; }
            }
            return;
        }
        if let Some(table) = self.0.weak_keys.get() {
            table.entries.borrow_mut().remove(&name.downgrade());
            return;
        }
        self.base().values_mut(mc).remove(&DynamicKey::Object(name));
    }

    pub fn has_property_by_object(self, name: Object<'gc>) -> bool {
        if let Some(owner) = self.owner(name) {
            return owner.display_owner_value(self.owner_id()).is_some();
        }
        if let Some(table) = self.0.weak_keys.get() {
            return table.entries.borrow().contains_key(&name.downgrade());
        }
        self.base().values().contains_key(&DynamicKey::Object(name))
    }
}

impl<'gc> TObject<'gc> for DictionaryObject<'gc> {
    fn gc_base(&self) -> Gc<'gc, ScriptObjectData<'gc>> {
        HasPrefixField::as_prefix_gc(self.0)
    }

    fn get_next_enumerant(self, last_index: u32, activation: &mut Activation<'_, 'gc>) -> Result<u32, Error<'gc>> {
        if last_index & (WEAK | OWNER) == 0 {
            let next = self.base().get_next_enumerant(last_index);
            if next != 0 { return Ok(next); }
        }
        if last_index & OWNER == 0 && let Some(table) = self.0.weak_keys.get() {
            let entries = table.entries.borrow();
            let mut index = if last_index & WEAK == 0 { 0 } else { (last_index & !WEAK) as usize };
            while let Some(next) = entries.next(index) {
                if entries.key_at(next).and_then(|key| key.upgrade(activation.gc())).is_some() {
                    return Ok(WEAK | u32::try_from(next).expect("dictionary enumeration overflow"));
                }
                index = next;
            }
        }
        let start = if last_index & OWNER == 0 { 0 } else { (last_index & !OWNER) as usize };
        let keys = self.0.owner_keys.borrow();
        for (index, key) in keys.iter().enumerate().skip(start) {
            if let Some(key) = key.and_then(|key| key.0.upgrade(activation.gc())) {
                if StageObject(key).display_owner_value(self.owner_id()).is_some() {
                    return Ok(OWNER | u32::try_from(index + 1).expect("dictionary enumeration overflow"));
                }
            }
        }
        Ok(0)
    }

    fn get_enumerant_name(self, index: u32, activation: &mut Activation<'_, 'gc>) -> Result<Value<'gc>, Error<'gc>> {
        if index & WEAK != 0 {
            let key = self.0.weak_keys.get().and_then(|table| {
                table.entries.borrow().key_at((index & !WEAK) as usize).copied()
            });
            return Ok(key.and_then(|key| key.upgrade(activation.gc())).map_or(Value::Null, Value::from));
        }
        if index & OWNER != 0 {
            let key = (index & 0x3fff_ffff).checked_sub(1).and_then(|i| self.0.owner_keys.borrow().get(i as usize).copied().flatten());
            return Ok(key.and_then(|key| key.0.upgrade(activation.gc())).map(|key| StageObject(key).into()).unwrap_or(Value::Null));
        }
        Ok(self.base().get_enumerant_name(index).unwrap_or(Value::Null))
    }

    // Calling `setPropertyIsEnumerable` on a `Dictionary` has no effect -
    // stringified properties are always enumerable.
    fn set_local_property_is_enumerable(
        &self,
        _mc: &Mutation<'gc>,
        _name: AvmString<'gc>,
        _is_enumerable: bool,
    ) {
    }

    fn get_enumerant_value(
        self,
        index: u32,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        if index & (WEAK | OWNER) != 0 {
            return Ok(self.get_enumerant_name(index, activation)?.as_object()
                .map(|key| self.get_property_by_object(key, activation.gc())).unwrap_or(Value::Undefined));
        }
        Ok(*self
            .base()
            .values()
            .value_at(index as usize)
            .unwrap_or(&Value::Undefined))
    }
}

#[cfg(test)]
mod owner_dictionary_tests {
    use super::*;
    use crate::avm2::globals::flash::display::sprite::sprite_allocator;
    use crate::avm2::object::ScriptObject;
    use crate::avm2::function::FunctionArgs;
    use crate::player::PlayerBuilder;

    #[test]
    fn aola_dictionary_amf3_roundtrip_preserves_key_mode_and_live_entries() {
        use crate::avm2::amf::{deserialize_value, serialize_value};
        use flash_lso::types::AMFVersion;

        let player = PlayerBuilder::new().build();
        player.lock().unwrap().mutate_with_update_context(|context| {
            let mut activation = Activation::from_nothing(context);
            let class = activation.avm2().classes().dictionary;
            for weak in [false, true] {
                let original = class.construct(&mut activation, &[weak.into()]).unwrap();
                let dictionary = original.as_object().unwrap().as_dictionary_object().unwrap();
                let key = ScriptObject::new_object(activation.context);
                dictionary.set_property_by_object(key, Value::Integer(42), activation.gc());
                let packed = serialize_value(&mut activation, original, AMFVersion::AMF3, &mut Default::default());
                let copy = deserialize_value(&mut activation, &packed).unwrap()
                    .as_object().unwrap().as_dictionary_object().unwrap();
                assert_eq!(copy.has_weak_keys(), weak);
                let index = copy.get_next_enumerant(0, &mut activation).unwrap();
                assert_ne!(index, 0);
                assert!(copy.get_enumerant_name(index, &mut activation).unwrap().as_object().is_some());
                assert!(matches!(copy.get_enumerant_value(index, &mut activation).unwrap(), Value::Integer(42)));
                assert_eq!(copy.get_next_enumerant(index, &mut activation).unwrap(), 0);
            }
        });
    }

    #[test]
    fn owner_dictionary_migrates_gets_enumerates_overwrites_and_deletes() {
        for weak in [false, true] {
            check_display_owners(weak);
        }
    }

    fn check_display_owners(weak: bool) {
        let player = PlayerBuilder::new().build();
        player.lock().unwrap().mutate_with_update_context(|context| {
            let mut activation = Activation::from_nothing(context);
            let dictionary_class = activation.avm2().classes().dictionary;
            let sprite_class = activation.avm2().classes().sprite;
            let dictionary = dictionary_allocator(dictionary_class, &mut activation).unwrap().as_dictionary_object().unwrap();
            let display = sprite_allocator(sprite_class, &mut activation).unwrap();
            let ordinary = ScriptObject::new_object(activation.context);
            dictionary.set_property_by_object(display, Value::Integer(17), activation.gc());
            dictionary.set_property_by_object(ordinary, Value::Null, activation.gc());
            if weak {
                dictionary.enable_weak_keys(&mut activation);
            }
            let method = AvmString::new_utf8(activation.gc(), "aoqiUseDisplayOwners");
            let returned = Value::from(dictionary).call_public_property(method, FunctionArgs::empty(), &mut activation).unwrap();
            assert_eq!(returned.as_object(), Some(dictionary.into()));
            dictionary.use_display_owners(activation.gc());
            assert!(matches!(dictionary.get_property_by_object(display, activation.gc()), Value::Integer(17)));
            assert!(dictionary.has_property_by_object(ordinary));
            dictionary.set_property_by_object(display, Value::Integer(23), activation.gc());
            let mut index = 0;
            let mut names = Vec::new();
            loop {
                index = dictionary.get_next_enumerant(index, &mut activation).unwrap();
                if index == 0 { break; }
                assert!((index as i32) > 0, "AVM2 hasnext rejects negative indices");
                let name = dictionary.get_enumerant_name(index, &mut activation).unwrap();
                if name.as_object() == Some(display) {
                    assert!(matches!(dictionary.get_enumerant_value(index, &mut activation).unwrap(), Value::Integer(23)));
                }
                names.push(name.as_object().unwrap());
            }
            assert_eq!(names.len(), 2);
            assert!(names.contains(&display));
            assert!(names.contains(&ordinary));
            dictionary.delete_property_by_object(display, activation.gc());
            assert!(!dictionary.has_property_by_object(display));
            assert!(matches!(dictionary.get_property_by_object(display, activation.gc()), Value::Undefined));
            dictionary.delete_property_by_object(ordinary, activation.gc());
            assert_eq!(dictionary.get_next_enumerant(0, &mut activation).unwrap(), 0);
        });
    }
}
