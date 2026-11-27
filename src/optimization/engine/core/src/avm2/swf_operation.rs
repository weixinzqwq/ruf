//! Observable method returns in the verified backpack and battle ABCs.
use std::sync::OnceLock;
use swf::avm2::types::{AbcFile, Index, Multiname, Namespace, TraitKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operation {
    Other,
    ConstructPanel,
    ClassifyView,
    DisposeSkin,
    DisposePanel,
    ClickView,
    ChangeStyle,
    ConstructFight,
    BattleEnableClick,
    BattleDisableClick,
    BattleInitPet,
    DisposeFightManagers,
    DisposeFight,
    SwitchPet,
    OpenActivityPanel,
}

impl Operation {
    /// Bind the FightPanel methods once for this ABC, before any AVM calls.
    pub(super) fn battle_methods(abc: &AbcFile) -> Box<[(u32, Self)]> {
        let pool = &abc.constant_pool;
        let qname = |id: Index<Multiname>| pool.multinames.get(id.0.checked_sub(1)? as usize);
        let string = |id: Index<String>| pool.strings.get(id.0.checked_sub(1)? as usize).map(Vec::as_slice);
        let owner = abc.instances.iter().find(|owner| {
            let Some(Multiname::QName { namespace, name }) = qname(owner.name) else { return false };
            let Some(Namespace::Package(package)) = namespace.0.checked_sub(1)
                .and_then(|id| pool.namespaces.get(id as usize)) else { return false };
            string(*package) == Some(b"mmo.petfightnew.common")
                && string(*name) == Some(b"FightPanel")
        });
        let Some(owner) = owner else { return Box::default() };
        let mut methods = vec![(owner.init_method.0, Self::ConstructFight)];
        for member in &owner.traits {
            let TraitKind::Method { method, .. } = member.kind else { continue };
            let Some(Multiname::QName { name, .. }) = qname(member.name) else { continue };
            let operation = match string(*name) {
                Some(b"enableClick") => Self::BattleEnableClick,
                Some(b"disableClick") => Self::BattleDisableClick,
                Some(b"initPet") => Self::BattleInitPet,
                Some(b"dispose") => Self::DisposeFight,
                Some(b"disposeManagers") => Self::DisposeFightManagers,
                _ => continue,
            };
            methods.push((method.0, operation));
        }
        methods.into_boxed_slice()
    }

    fn sampling_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var_os("AOLA_SWF_OPERATIONS").is_some_and(|v| v == "1"))
    }

    pub(super) fn tracks_battle_lifecycle() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| Self::sampling_enabled()
            || ruffle_render::backend::aola_long_run_maintenance()
            || crate::goto_profile::goto_target_only_mode() == "battle"
            || crate::orphan_manager::ordinary_orphan_mode() == "battle")
    }

    pub(super) fn for_method(abc: &AbcFile, index: u32, battle_methods: &[(u32, Self)]) -> Self {
        if let Some((_, operation)) = battle_methods.iter().find(|(id, _)| *id == index) {
            return *operation;
        }
        // Check the cheap method index before inspecting the owner and binding.
        let (operation, instance, package, class, member) = match index {
            // Method bindings are cached before the tools window can enable capture.
            14 => (Self::OpenActivityPanel, 0, "mmo.play.activity.helper", "Panel_Helper", "loadResAndShow"),
            222 => (Self::ClassifyView, 28, "mmo.petpackage.controller.skin", "PackageSkinManager", "changeClassify"),
            224 => (Self::DisposeSkin, 28, "mmo.petpackage.controller.skin", "PackageSkinManager", "dispose"),
            317 => (Self::ConstructPanel, 41, "mmo.petpackage.panel", "PetPackagePanel", ""),
            375 => (Self::DisposePanel, 41, "mmo.petpackage.panel", "PetPackagePanel", "disposeEvent"),
            1231 => (Self::ClickView, 128, "mmo.petpackage.panel", "SimpleViewController", "onClickMe"),
            1232 => (Self::ChangeStyle, 128, "mmo.petpackage.panel", "SimpleViewController", "changeStyle"),
            1646 if crate::pet_switch_frame::enabled() => (Self::SwitchPet, 143, "mmo.petfightnew.common.fightcontroller", "SwitchPm", "switchPet"),
            _ => return Self::Other,
        };
        if !(matches!(operation, Self::SwitchPet | Self::OpenActivityPanel) || Self::sampling_enabled()) {
            return Self::Other;
        }
        let Some(owner) = abc.instances.get(instance) else { return Self::Other };
        let pool = &abc.constant_pool;
        let qname = |id: Index<Multiname>| pool.multinames.get(id.0.checked_sub(1)? as usize);
        let string = |id: Index<String>| pool.strings.get(id.0.checked_sub(1)? as usize).map(Vec::as_slice);
        let Some(Multiname::QName { namespace, name }) = qname(owner.name) else { return Self::Other };
        let Some(Namespace::Package(uri)) = namespace.0.checked_sub(1).and_then(|i| pool.namespaces.get(i as usize)) else {
            return Self::Other;
        };
        if string(*uri) != Some(package.as_bytes()) || string(*name) != Some(class.as_bytes()) {
            return Self::Other;
        }
        let bound = if operation == Self::OpenActivityPanel {
            let Some(class) = abc.classes.get(instance) else { return Self::Other };
            let Some(method) = abc.methods.get(index as usize) else { return Self::Other };
            // Official play~261031358096721 helper ABC: static binding and complete six-parameter signature.
            let typename = |key| match qname(key)? { Multiname::QName { name, .. } => string(*name), _ => None };
            let types: [&[u8]; 6] = [b"String", b"String", b"Object", b"ResDomain", b"Function", b"int"];
            method.params.len() == types.len() && typename(method.return_type) == Some(&b"void"[..])
                && method.params.iter().zip(types).all(|(param, name)| typename(param.kind) == Some(name))
                && class.traits.iter().any(|t| matches!(t.kind, TraitKind::Method { method, .. } if method.0 == index)
                    && matches!(qname(t.name), Some(Multiname::QName { name, .. }) if string(*name) == Some(member.as_bytes())))
        } else if operation == Self::ConstructPanel {
            owner.init_method.0 == index
        } else {
            owner.traits.iter().any(|t| matches!(t.kind, TraitKind::Method { method, .. } if method.0 == index)
                && matches!(qname(t.name), Some(Multiname::QName { name, .. }) if string(*name) == Some(member.as_bytes())))
        };
        if bound { operation } else { Self::Other }
    }

    pub(super) fn observes_return(self) -> bool {
        matches!(self, Self::ConstructPanel | Self::ClassifyView | Self::DisposeSkin)
    }

    pub(super) fn event(self, parent: Self, grandparent: Self) -> Option<&'static str> {
        match self {
            Self::ConstructPanel => Some("panel-created"),
            Self::ClassifyView if parent == Self::ChangeStyle && grandparent == Self::ClickView => Some("view-switch"),
            Self::ClassifyView => Some("classify-return"),
            // disposeEvent's reentrant/already-disposed path returns before this call.
            Self::DisposeSkin if parent == Self::DisposePanel => Some("panel-cleanup"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Operation;
    use super::Operation::*;

    #[test]
    fn aola_battle_input_operations_bind_shifted_methods_by_name() {
        use swf::avm2::types::{AbcFile, ConstantPool, Index, Instance, Multiname, Namespace, Trait, TraitKind};

        let qname = |namespace, name| Multiname::QName {
            namespace: Index::new(namespace),
            name: Index::new(name),
        };
        let member = |name, id| Trait {
            name: Index::new(name),
            kind: TraitKind::Method { disp_id: 0, method: Index::new(id) },
            metadata: vec![],
            is_final: false,
            is_override: false,
        };
        let owner = Instance {
            name: Index::new(3),
            super_name: Index::new(0),
            is_sealed: false,
            is_final: false,
            is_interface: false,
            protected_namespace: None,
            interfaces: vec![],
            init_method: Index::new(19),
            traits: vec![member(4, 20), member(5, 39), member(6, 38), member(7, 58),
                member(8, 40), member(9, 81)],
        };
        let other_class = Instance { name: Index::new(2), init_method: Index::new(101), ..owner.clone() };
        let wrong_package = Instance { name: Index::new(1), init_method: Index::new(111), ..owner.clone() };
        let abc = AbcFile {
            major_version: 46,
            minor_version: 16,
            constant_pool: ConstantPool {
                ints: vec![],
                uints: vec![],
                doubles: vec![],
                strings: ["other.package", "mmo.petfightnew.common", "FightPanel", "OtherClass",
                    "enableClick", "disableClick", "dispose", "disposeManagers", "notAnOperation", "initPet"]
                    .into_iter().map(|name| name.as_bytes().to_vec()).collect(),
                namespaces: vec![Namespace::Package(Index::new(1)), Namespace::Package(Index::new(2)),
                    Namespace::Package(Index::new(0))],
                namespace_sets: vec![],
                multinames: vec![qname(1, 3), qname(2, 4), qname(2, 3), qname(3, 5),
                    qname(3, 6), qname(3, 7), qname(3, 8), qname(3, 9), qname(3, 10)],
            },
            methods: vec![],
            metadata: vec![],
            instances: vec![wrong_package, other_class, owner],
            classes: vec![],
            scripts: vec![],
            method_bodies: vec![],
        };
        let methods = Operation::battle_methods(&abc);
        for (index, expected) in [(19, ConstructFight), (20, BattleEnableClick),
            (39, BattleDisableClick), (38, DisposeFight), (58, DisposeFightManagers), (81, BattleInitPet),
            (40, Other), (101, Other), (111, Other), (4023, Other)] {
            assert_eq!(Operation::for_method(&abc, index, &methods), expected);
        }
    }

    #[test]
    fn aola_swf_operation_requires_the_game_call_chain() {
        assert_eq!(ClassifyView.event(ChangeStyle, ClickView), Some("view-switch"));
        assert_eq!(ClassifyView.event(ChangeStyle, Other), Some("classify-return"));
        assert_eq!(ClassifyView.event(Other, ClickView), Some("classify-return"));
        assert_eq!(DisposeSkin.event(DisposePanel, Other), Some("panel-cleanup"));
        assert_eq!(DisposeSkin.event(Other, Other), None);
        assert_eq!(DisposePanel.event(Other, Other), None);
        assert_eq!(ConstructPanel.event(Other, Other), Some("panel-created"));
    }

    #[test]
    #[ignore = "Set AOLA_TEST_BATTLE_FILE to an actual petfightnew SWF"]
    fn battle_operations_bind_to_real_abc() {
        use swf::avm2::read::Reader;
        use swf::avm2::types::{Index, Multiname, Op, TraitKind};
        use swf::extensions::ReadSwfExt;

        let path = std::env::var_os("AOLA_TEST_BATTLE_FILE").unwrap();
        let data = std::fs::read(path).unwrap();
        let buffer = swf::decompress_swf(data.as_slice()).unwrap();
        let movie = swf::parse_swf(&buffer).unwrap();
        let (mut abc, methods) = movie.tags.iter().find_map(|tag| {
            let swf::Tag::DoAbc2(tag) = tag else { return None };
            let abc = Reader::new(tag.data).read().unwrap();
            let methods = Operation::battle_methods(&abc);
            (!methods.is_empty()).then_some((abc, methods))
        }).expect("FightPanel constructor");
        let binding = |target| methods.iter().find(|(_, op)| *op == target).unwrap().0;
        let constructor = binding(ConstructFight);
        let manager = binding(DisposeFightManagers);
        let dispose = binding(DisposeFight);
        assert_eq!(Operation::for_method(&abc, binding(BattleEnableClick), &methods), BattleEnableClick);
        assert_eq!(Operation::for_method(&abc, binding(BattleDisableClick), &methods), BattleDisableClick);
        assert_eq!(Operation::for_method(&abc, binding(BattleInitPet), &methods), BattleInitPet);

        let body = &abc.method_bodies[abc.methods[dispose as usize].body.unwrap().0 as usize];
        let mut code = Reader::new(&body.code);
        let (mut manager_calls, mut early_return, mut completed_return) = (0, false, false);
        while !code.as_slice().is_empty() {
            match code.read_op().unwrap() {
                Op::CallPropVoid { index, num_args: 0 } => {
                    let name = match &abc.constant_pool.multinames[index.0 as usize - 1] {
                        Multiname::QName { name, .. } | Multiname::Multiname { name, .. } => name,
                        _ => continue,
                    };
                    if abc.constant_pool.strings[name.0 as usize - 1] == b"disposeManagers" {
                        manager_calls += 1;
                    }
                }
                Op::ReturnVoid if manager_calls == 0 => early_return = true,
                Op::ReturnVoid => completed_return = true,
                _ => {}
            }
        }
        assert_eq!((manager_calls, early_return, completed_return), (1, true, true));
        let owner = abc.instances.iter().position(|owner| owner.init_method.0 == constructor).unwrap();
        abc.instances[owner].init_method = Index::new(0);
        assert_eq!(Operation::for_method(&abc, constructor, &Operation::battle_methods(&abc)), Other);
        abc.instances[owner].traits.retain(|t| !matches!(t.kind, TraitKind::Method { method, .. } if method.0 == manager));
        assert_eq!(Operation::for_method(&abc, manager, &Operation::battle_methods(&abc)), Other);
        abc.instances[owner].name = Index::new(0);
        assert_eq!(Operation::for_method(&abc, dispose, &Operation::battle_methods(&abc)), Other);
    }
}
