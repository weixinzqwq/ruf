//! Observable game method returns, bound by name when the ABC loads.
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
    pub(super) fn bind_methods(abc: &AbcFile) -> Box<[(u32, Self)]> {
        Self::bind_methods_with_options(
            abc,
            Self::sampling_enabled(),
            crate::pet_switch_frame::enabled(),
        )
    }

    fn bind_methods_with_options(
        abc: &AbcFile,
        sampling: bool,
        pet_switch_timing: bool,
    ) -> Box<[(u32, Self)]> {
        let pool = &abc.constant_pool;
        let qname = |id: Index<Multiname>| pool.multinames.get(id.0.checked_sub(1)? as usize);
        let string = |id: Index<String>| pool.strings.get(id.0.checked_sub(1)? as usize).map(Vec::as_slice);
        let typename = |id| match qname(id)? {
            Multiname::QName { name, .. } => string(*name),
            _ => None,
        };
        let mut methods = Vec::new();
        for (class_index, owner) in abc.instances.iter().enumerate() {
            let Some(Multiname::QName { namespace, name }) = qname(owner.name) else { continue };
            let Some(Namespace::Package(package)) = namespace.0.checked_sub(1)
                .and_then(|id| pool.namespaces.get(id as usize)) else { continue };
            let mut traits = owner.traits.as_slice();
            let (constructor, members): (Option<Self>, &[(&[u8], Self)]) =
                match (string(*package), string(*name)) {
                    (Some(b"mmo.petfightnew.common"), Some(b"FightPanel")) => (
                        Some(Self::ConstructFight),
                        &[
                            (b"enableClick", Self::BattleEnableClick),
                            (b"disableClick", Self::BattleDisableClick),
                            (b"initPet", Self::BattleInitPet),
                            (b"dispose", Self::DisposeFight),
                            (b"disposeManagers", Self::DisposeFightManagers),
                        ],
                    ),
                    (Some(b"mmo.play.activity.helper"), Some(b"Panel_Helper")) => {
                        // Capture can be enabled after this ABC is loaded.
                        let Some(class) = abc.classes.get(class_index) else { continue };
                        traits = &class.traits;
                        (None, &[(b"loadResAndShow", Self::OpenActivityPanel)])
                    }
                    (Some(b"mmo.petfightnew.common.fightcontroller"), Some(b"SwitchPm"))
                        if pet_switch_timing =>
                    {
                        (None, &[(b"switchPet", Self::SwitchPet)])
                    }
                    (Some(b"mmo.petpackage.controller.skin"), Some(b"PackageSkinManager"))
                        if sampling =>
                    {
                        (None, &[(b"changeClassify", Self::ClassifyView), (b"dispose", Self::DisposeSkin)])
                    }
                    (Some(b"mmo.petpackage.panel"), Some(b"PetPackagePanel")) if sampling => (
                        Some(Self::ConstructPanel),
                        &[(b"disposeEvent", Self::DisposePanel)],
                    ),
                    (Some(b"mmo.petpackage.panel"), Some(b"SimpleViewController")) if sampling => (
                        None,
                        &[(b"onClickMe", Self::ClickView), (b"changeStyle", Self::ChangeStyle)],
                    ),
                    _ => continue,
            };
            if let Some(operation) = constructor {
                methods.push((owner.init_method.0, operation));
            }
            for member in traits {
                let TraitKind::Method { method, .. } = member.kind else { continue };
                let Some(name) = typename(member.name) else { continue };
                let Some(&(_, operation)) = members.iter().find(|(member, _)| *member == name) else { continue };
                if operation == Self::OpenActivityPanel {
                    let Some(signature) = abc.methods.get(method.0 as usize) else { continue };
                    let types: [&[u8]; 6] = [b"String", b"String", b"Object", b"ResDomain", b"Function", b"int"];
                    if signature.params.len() != types.len()
                        || typename(signature.return_type) != Some(b"void")
                        || !signature.params.iter().zip(types).all(|(param, name)| typename(param.kind) == Some(name))
                    {
                        continue;
                    }
                }
                methods.push((method.0, operation));
            }
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

    pub(super) fn for_method(index: u32, methods: &[(u32, Self)]) -> Self {
        methods.iter().find_map(|&(id, operation)| (id == index).then_some(operation))
            .unwrap_or(Self::Other)
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
        let methods = Operation::bind_methods(&abc);
        for (index, expected) in [(19, ConstructFight), (20, BattleEnableClick),
            (39, BattleDisableClick), (38, DisposeFight), (58, DisposeFightManagers), (81, BattleInitPet),
            (40, Other), (101, Other), (111, Other), (4023, Other)] {
            assert_eq!(Operation::for_method(index, &methods), expected);
        }
    }

    #[test]
    fn aola_game_operations_bind_shifted_methods_by_name() {
        use swf::avm2::types::{
            AbcFile, Class, ConstantPool, Index, Instance, Method, MethodFlags, MethodParam,
            Multiname, Namespace, Trait, TraitKind,
        };

        let mut pool = ConstantPool {
            ints: vec![],
            uints: vec![],
            doubles: vec![],
            strings: vec![],
            namespaces: vec![],
            namespace_sets: vec![],
            multinames: vec![],
        };
        let mut qname = |package: &str, name: &str| {
            pool.strings.push(package.as_bytes().to_vec());
            pool.namespaces.push(Namespace::Package(Index::new(pool.strings.len() as u32)));
            pool.strings.push(name.as_bytes().to_vec());
            pool.multinames.push(Multiname::QName {
                namespace: Index::new(pool.namespaces.len() as u32),
                name: Index::new(pool.strings.len() as u32),
            });
            Index::new(pool.multinames.len() as u32)
        };
        let member = |name, method| Trait {
            name,
            kind: TraitKind::Method { disp_id: 0, method: Index::new(method) },
            metadata: vec![],
            is_final: false,
            is_override: false,
        };
        let class = |name, constructor, traits| Instance {
            name,
            super_name: Index::new(0),
            is_sealed: false,
            is_final: false,
            is_interface: false,
            protected_namespace: None,
            interfaces: vec![],
            init_method: Index::new(constructor),
            traits,
        };
        let load = qname("", "loadResAndShow");
        let instances = vec![
            class(qname("mmo.play.activity.helper", "Panel_Helper"), 0, vec![member(load, 9)]),
            class(qname("mmo.petfightnew.common.fightcontroller", "SwitchPm"), 0,
                vec![member(qname("", "switchPet"), 2)]),
            class(qname("mmo.petpackage.controller.skin", "PackageSkinManager"), 0,
                vec![member(qname("", "changeClassify"), 3), member(qname("", "dispose"), 4)]),
            class(qname("mmo.petpackage.panel", "PetPackagePanel"), 5,
                vec![member(qname("", "disposeEvent"), 6)]),
            class(qname("mmo.petpackage.panel", "SimpleViewController"), 0,
                vec![member(qname("", "onClickMe"), 7), member(qname("", "changeStyle"), 8)]),
        ];
        let mut classes = vec![Class { init_method: Index::new(0), traits: vec![] }; instances.len()];
        classes[0].traits.push(member(load, 1));
        let empty = Method {
            name: Index::new(0),
            return_type: qname("", "void"),
            params: vec![],
            flags: MethodFlags::empty(),
            body: None,
        };
        let mut methods = vec![empty.clone(); 10];
        methods[1].params = ["String", "String", "Object", "ResDomain", "Function", "int"]
            .map(|name| MethodParam { kind: qname("", name), name: None, default_value: None })
            .into();
        methods[9] = methods[1].clone();
        let mut abc = AbcFile {
            major_version: 46,
            minor_version: 16,
            constant_pool: pool,
            methods,
            metadata: vec![],
            instances,
            classes,
            scripts: vec![],
            method_bodies: vec![],
        };

        let expected = [OpenActivityPanel, SwitchPet, ClassifyView, DisposeSkin, ConstructPanel,
            DisposePanel, ClickView, ChangeStyle];
        let bindings = Operation::bind_methods_with_options(&abc, true, true);
        for (offset, operation) in expected.into_iter().enumerate() {
            assert_eq!(Operation::for_method(offset as u32 + 1, &bindings), operation);
        }
        assert_eq!(Operation::for_method(9, &bindings), Other); // Same name and signature, instance trait.

        let no_sampling = Operation::bind_methods_with_options(&abc, false, true);
        assert_eq!(no_sampling.as_ref(), &[(1, OpenActivityPanel), (2, SwitchPet)]);
        let no_switch_timing = Operation::bind_methods_with_options(&abc, true, false);
        assert_eq!(Operation::for_method(2, &no_switch_timing), Other);

        let shift = 73;
        abc.methods.splice(0..0, std::iter::repeat_n(empty, shift as usize));
        abc.instances.rotate_left(1);
        abc.classes.rotate_left(1);
        for owner in &mut abc.instances {
            owner.init_method.0 += shift;
        }
        let traits = abc.instances.iter_mut().flat_map(|owner| &mut owner.traits)
            .chain(abc.classes.iter_mut().flat_map(|owner| &mut owner.traits));
        for member in traits {
            if let TraitKind::Method { method, .. } = &mut member.kind {
                method.0 += shift;
            }
        }
        let bindings = Operation::bind_methods_with_options(&abc, true, true);
        for (offset, operation) in expected.into_iter().enumerate() {
            let original = offset as u32 + 1;
            assert_eq!(Operation::for_method(original + shift, &bindings), operation);
            assert_eq!(Operation::for_method(original, &bindings), Other);
        }

        let activity = &mut abc.methods[shift as usize + 1];
        activity.params[5].kind = activity.params[0].kind;
        let invalid = Operation::bind_methods_with_options(&abc, true, true);
        assert_eq!(Operation::for_method(shift + 1, &invalid), Other);
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
            let methods = Operation::bind_methods(&abc);
            methods.iter().any(|&(_, op)| op == ConstructFight).then_some((abc, methods))
        }).expect("FightPanel constructor");
        let binding = |target| methods.iter().find(|(_, op)| *op == target).unwrap().0;
        let constructor = binding(ConstructFight);
        let manager = binding(DisposeFightManagers);
        let dispose = binding(DisposeFight);
        assert_eq!(Operation::for_method(binding(BattleEnableClick), &methods), BattleEnableClick);
        assert_eq!(Operation::for_method(binding(BattleDisableClick), &methods), BattleDisableClick);
        assert_eq!(Operation::for_method(binding(BattleInitPet), &methods), BattleInitPet);

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
        assert_eq!(Operation::for_method(constructor, &Operation::bind_methods(&abc)), Other);
        abc.instances[owner].traits.retain(|t| !matches!(t.kind, TraitKind::Method { method, .. } if method.0 == manager));
        assert_eq!(Operation::for_method(manager, &Operation::bind_methods(&abc)), Other);
        abc.instances[owner].name = Index::new(0);
        assert_eq!(Operation::for_method(dispose, &Operation::bind_methods(&abc)), Other);
    }
}
