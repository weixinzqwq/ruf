//! Object representation for `flash.utils.Dictionary`

use crate::avm2::Error;
use crate::avm2::activation::Activation;
use crate::avm2::dynamic_map::DynamicKey;
use crate::avm2::object::script_object::ScriptObjectData;
use crate::avm2::object::{ClassObject, Object, TObject, StageObject, StageObjectWeak};
use crate::avm2::value::Value;
use crate::string::AvmString;
use core::fmt;
use gc_arena::{Collect, Gc, GcWeak, Mutation};
use gc_arena::{barrier::unlock, lock::RefLock};
use std::cell::Cell;
use ruffle_common::utils::HasPrefixField;

/// A class instance allocator that allocates Dictionary objects.
pub fn dictionary_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    let base = ScriptObjectData::new(class);

    Ok(DictionaryObject(Gc::new(activation.gc(), DictionaryObjectData {
        base,
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
    #[collect(require_static)]
    display_owners: Cell<bool>,
    owner_keys: RefLock<Vec<Option<StageObjectWeak<'gc>>>>,
}

impl<'gc> DictionaryObject<'gc> {
    fn owner_id(self) -> GcWeak<'gc, ()> {
        GcWeak::erase(Gc::downgrade(self.0))
    }

    /// Explicit opt-in extension used by the recovered FairyGUI bridge.
    /// Non-display keys continue using the ordinary upstream dictionary path.
    pub fn use_display_owners(self, mc: &Mutation<'gc>) {
        if self.0.display_owners.replace(true) { return; }
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
    pub fn get_property_by_object(self, name: Object<'gc>) -> Value<'gc> {
        if let Some(owner) = self.owner(name) {
            return owner.display_owner_value(self.owner_id()).unwrap_or(Value::Undefined);
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
        self.base().values_mut(mc).remove(&DynamicKey::Object(name));
    }

    pub fn has_property_by_object(self, name: Object<'gc>) -> bool {
        if let Some(owner) = self.owner(name) {
            return owner.display_owner_value(self.owner_id()).is_some();
        }
        self.base().values().contains_key(&DynamicKey::Object(name))
    }
}

impl<'gc> TObject<'gc> for DictionaryObject<'gc> {
    fn gc_base(&self) -> Gc<'gc, ScriptObjectData<'gc>> {
        HasPrefixField::as_prefix_gc(self.0)
    }

    fn get_next_enumerant(self, last_index: u32, activation: &mut Activation<'_, 'gc>) -> Result<u32, Error<'gc>> {
        // Bit 30 identifies owner slots; indices stay positive i32 for AVM2
        // hasnext/hasnext2. This keeps ordinary string and
        // non-display object enumeration independent of weak-key tombstones.
        const OWNER: u32 = 0x4000_0000;
        if last_index & OWNER == 0 {
            let next = self.base().get_next_enumerant(last_index);
            if next != 0 { return Ok(next); }
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
        if index & 0x4000_0000 != 0 {
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
        if index & 0x4000_0000 != 0 {
            return Ok(self.get_enumerant_name(index, activation)?.as_object()
                .map(|key| self.get_property_by_object(key)).unwrap_or(Value::Undefined));
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
    fn owner_dictionary_migrates_gets_enumerates_overwrites_and_deletes() {
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
            let method = AvmString::new_utf8(activation.gc(), "aoqiUseDisplayOwners");
            let returned = Value::from(dictionary).call_public_property(method, FunctionArgs::empty(), &mut activation).unwrap();
            assert_eq!(returned.as_object(), Some(dictionary.into()));
            dictionary.use_display_owners(activation.gc());
            assert!(matches!(dictionary.get_property_by_object(display), Value::Integer(17)));
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
            assert!(matches!(dictionary.get_property_by_object(display), Value::Undefined));
            dictionary.delete_property_by_object(ordinary, activation.gc());
            assert_eq!(dictionary.get_next_enumerant(0, &mut activation).unwrap(), 0);
        });
    }
}
