//! AVM2 object impl for the display hierarchy.

use crate::avm2::Error;
use crate::avm2::activation::Activation;
use crate::avm2::function::FunctionArgs;
use crate::avm2::object::script_object::ScriptObjectData;
use crate::avm2::object::{ClassObject, TObject};
use crate::avm2::value::Value;
use crate::display_object::DisplayObject;
use gc_arena::{Collect, Gc, GcWeak, Mutation, collect::Trace};
use std::cell::RefCell;
use ruffle_common::utils::HasPrefixField;
use std::fmt::Debug;

#[derive(Clone, Collect, Copy)]
#[collect(no_drop)]
pub struct StageObject<'gc>(pub Gc<'gc, StageObjectData<'gc>>);

#[derive(Clone, Collect, Copy, Debug)]
#[collect(no_drop)]
pub struct StageObjectWeak<'gc>(pub GcWeak<'gc, StageObjectData<'gc>>);

#[derive(Clone, Collect, HasPrefixField)]
#[collect(no_drop)]
#[repr(C, align(8))]
pub struct StageObjectData<'gc> {
    /// The base data common to all AVM2 objects.
    base: ScriptObjectData<'gc>,

    /// The associated display object.
    display_object: DisplayObject<'gc>,
    // A display object, not the dictionary, owns these values. Consequently a
    // value -> display back-reference does not make a global dictionary a root.
    owner_values: DisplayOwnerValues<'gc, Value<'gc>>,
}

#[derive(Clone)]
struct DisplayOwnerValues<'gc, V>(RefCell<Vec<(GcWeak<'gc, ()>, V)>>);

impl<V> Default for DisplayOwnerValues<'_, V> {
    fn default() -> Self { Self(RefCell::new(Vec::new())) }
}

// SAFETY: tracing only removes entries, then traces every remaining strong and
// weak pointer. Mutation which adopts pointers is private and is always preceded
// by the StageObject write barrier. No GC pointer is dereferenced while dropping.
unsafe impl<'gc, V: Collect<'gc>> Collect<'gc> for DisplayOwnerValues<'gc, V> {
    fn trace<C: Trace<'gc>>(&self, cc: &mut C) {
        let mut entries = self.0.borrow_mut();
        entries.retain(|(dictionary, _)| !dictionary.is_dropped());
        cc.trace(&*entries);
    }
}

impl<'gc> StageObject<'gc> {
    /// Allocate the AVM2 side of a display object intended to be of a given
    /// class's type.
    ///
    /// This function makes no attempt to construct the returned object. You
    /// are responsible for calling the native initializer of the given
    /// class at a later time. Typically, a display object that can contain
    /// movie-constructed children must first allocate itself (using this
    /// function), construct it's children, and then finally initialize itself.
    /// Display objects that do not need to use this flow should use
    /// `for_display_object_childless`.
    pub fn for_display_object(
        mc: &Mutation<'gc>,
        display_object: DisplayObject<'gc>,
        class: ClassObject<'gc>,
    ) -> Self {
        Self(Gc::new(
            mc,
            StageObjectData {
                base: ScriptObjectData::new(class),
                display_object,
                owner_values: DisplayOwnerValues::default(),
            },
        ))
    }

    /// Allocate and construct the AVM2 side of a display object intended to be
    /// of a given class's type.
    ///
    /// This function is intended for display objects that do not have children
    /// and thus do not need to be allocated and initialized in separate phases.
    pub fn for_display_object_childless(
        activation: &mut Activation<'_, 'gc>,
        display_object: DisplayObject<'gc>,
        class: ClassObject<'gc>,
    ) -> Result<Self, Error<'gc>> {
        let this = Self::for_display_object(activation.gc(), display_object, class);

        class.call_init(this.into(), FunctionArgs::empty(), activation)?;

        Ok(this)
    }

    /// Create a `graphics` object for a given display object.
    pub fn graphics(
        activation: &mut Activation<'_, 'gc>,
        display_object: DisplayObject<'gc>,
    ) -> Self {
        // note: for Graphics, there's no need to call init.

        let class = activation.avm2().classes().graphics;
        Self(Gc::new(
            activation.gc(),
            StageObjectData {
                base: ScriptObjectData::new(class),
                display_object,
                owner_values: DisplayOwnerValues::default(),
            },
        ))
    }

    pub fn display_object(self) -> DisplayObject<'gc> {
        self.0.display_object
    }

    pub fn display_owner_value(self, dictionary: GcWeak<'gc, ()>) -> Option<Value<'gc>> {
        self.0.owner_values.0.borrow().iter()
            .find(|(key, _)| GcWeak::ptr_eq(*key, dictionary)).map(|(_, value)| *value)
    }

    pub fn set_display_owner_value(self, dictionary: GcWeak<'gc, ()>, value: Value<'gc>, mc: &Mutation<'gc>) {
        let mut entries = Gc::write(mc, self.0).owner_values.0.borrow_mut();
        entries.retain(|(key, _)| !key.is_dropped());
        if let Some((_, old)) = entries.iter_mut().find(|(key, _)| GcWeak::ptr_eq(*key, dictionary)) {
            *old = value;
        } else {
            entries.push((dictionary, value));
        }
    }

    pub fn remove_display_owner_value(self, dictionary: GcWeak<'gc, ()>, mc: &Mutation<'gc>) {
        Gc::write(mc, self.0).owner_values.0.borrow_mut()
            .retain(|(key, _)| !GcWeak::ptr_eq(*key, dictionary) && !key.is_dropped());
    }
}

impl<'gc> TObject<'gc> for StageObject<'gc> {
    fn gc_base(&self) -> Gc<'gc, ScriptObjectData<'gc>> {
        HasPrefixField::as_prefix_gc(self.0)
    }
}

impl Debug for StageObject<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("StageObject")
            .field("name", &self.base().class_name())
            // .field("display_object", &self.0.display_object) TODO(moulins)
            .field("ptr", &Gc::as_ptr(self.0))
            .finish()
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;
    use gc_arena::{Arena, Rootable, lock::RefLock};

    #[derive(Collect)]
    #[collect(no_drop)]
    struct Node<'gc> {
        owners: DisplayOwnerValues<'gc, Gc<'gc, Node<'gc>>>,
        links: RefLock<Vec<Gc<'gc, Node<'gc>>>>,
    }
    type TestArena = Arena<Rootable![(Option<Gc<'_, ()>>, Option<Gc<'_, Node<'_>>>, GcWeak<'_, Node<'_>>, GcWeak<'_, Node<'_>>)]>;

    fn cycle() -> TestArena {
        TestArena::new(|mc| {
            let dictionary = Gc::new(mc, ());
            let owner = Gc::new(mc, Node { owners: DisplayOwnerValues::default(), links: RefLock::new(vec![]) });
            let value = Gc::new(mc, Node { owners: DisplayOwnerValues::default(), links: RefLock::new(vec![owner]) });
            Gc::write(mc, owner).owners.0.borrow_mut().push((Gc::downgrade(dictionary), value));
            (Some(dictionary), Some(owner), Gc::downgrade(owner), Gc::downgrade(value))
        })
    }

    #[test]
    fn live_display_keeps_owner_value_and_unrooted_cycle_is_collected() {
        let mut arena = cycle();
        arena.finish_cycle();
        arena.mutate(|mc, root| {
            assert!(root.2.upgrade(mc).is_some());
            assert!(root.3.upgrade(mc).is_some());
        });
        arena.mutate_root(|_, root| root.1 = None);
        arena.finish_cycle();
        arena.finish_cycle();
        arena.mutate(|_, root| {
            assert!(root.0.is_some());
            assert!(root.2.is_dropped());
            assert!(root.3.is_dropped());
        });
    }

    #[test]
    fn destroyed_dictionary_releases_value_even_while_display_survives() {
        let mut arena = cycle();
        arena.mutate_root(|_, root| root.0 = None);
        arena.finish_cycle();
        arena.finish_cycle();
        arena.mutate(|_, root| {
            assert!(!root.2.is_dropped());
            assert!(root.3.is_dropped());
            assert!(root.1.unwrap().owners.0.borrow().is_empty());
        });
    }
}
