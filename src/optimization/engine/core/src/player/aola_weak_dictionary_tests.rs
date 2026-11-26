use super::PlayerBuilder;
use crate::avm2::object::{DictionaryObject, ScriptObject, WeakObject};
use crate::avm2::{Activation, Object, TObject};
use gc_arena::{Gc, Rootable};

#[test]
fn aola_weak_dictionary_follows_live_chains_and_prunes_idle_tables() {
    let player = PlayerBuilder::new().build();
    let mut player = player.lock().unwrap();
    let (tables, keep_key) = player.mutate_with_update_context(|context| {
        let mut activation = Activation::from_nothing(context);
        let class = activation.avm2().classes().dictionary;
        let mut make_weak = || class.construct(&mut activation, &[true.into()]).unwrap()
            .as_object().unwrap().as_dictionary_object().unwrap();
        let a = make_weak();
        let b = make_weak();
        assert!(a.has_weak_keys() && b.has_weak_keys());
        let keys = std::array::from_fn::<_, 4, _>(|_| ScriptObject::new_object(activation.context));
        let mc = activation.gc();
        for pair in keys.chunks_exact(2) {
            a.set_property_by_object(pair[0], pair[1].into(), mc);
            b.set_property_by_object(pair[1], pair[0].into(), mc);
        }
        let tables = activation.context.dynamic_root.stash::<Rootable![(
            DictionaryObject<'_>, DictionaryObject<'_>, [WeakObject<'_>; 4],
        )]>(mc, Gc::new(mc, (a, b, keys.map(Object::downgrade))));
        let keep_key = activation.context.dynamic_root.stash::<Rootable![Object<'_>]>(
            mc, Gc::new(mc, keys[0]),
        );
        (tables, keep_key)
    });

    // The tables remain rooted. No AS3 table operation runs between collections.
    player.gc_arena.borrow_mut().finish_cycle();
    player.gc_arena.borrow_mut().finish_cycle();
    player.mutate_with_update_context(|context| {
        let (a, b, keys) = *context.dynamic_root.fetch(&tables);
        let mc = context.gc();
        let first = keys[0].upgrade(mc).unwrap();
        let second = keys[1].upgrade(mc).unwrap();
        assert_eq!(a.get_property_by_object(first, mc).as_object(), Some(second));
        assert_eq!(b.get_property_by_object(second, mc).as_object(), Some(first));
        assert!(keys[2].is_dropped() && keys[3].is_dropped());
        let stats = context.avm2.weak_dictionary_stats.get();
        assert_eq!((stats.inserted, stats.pruned), (4, 2));
    });
    drop(keep_key);
    player.gc_arena.borrow_mut().finish_cycle();
    player.gc_arena.borrow_mut().finish_cycle();
    player.mutate_with_update_context(|context| {
        let (a, b, keys) = *context.dynamic_root.fetch(&tables);
        assert!(keys.iter().all(|key| key.is_dropped()));
        assert_eq!(context.avm2.weak_dictionary_stats.get().pruned, 4);
        let mut activation = Activation::from_nothing(context);
        assert_eq!(a.get_next_enumerant(0, &mut activation).unwrap(), 0);
        assert_eq!(b.get_next_enumerant(0, &mut activation).unwrap(), 0);
    });
}
