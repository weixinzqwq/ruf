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
    DisposeFightManagers,
    DisposeFight,
    SwitchPet,
    OpenActivityPanel,
}

impl Operation {
    pub(super) fn for_method(abc: &AbcFile, index: u32) -> Self {
        // Check the cheap method index before inspecting the owner and binding.
        let (operation, instance, package, class, member) = match index {
            14 if crate::mypro_observer::installed() => (Self::OpenActivityPanel, 0, "mmo.play.activity.helper", "Panel_Helper", "loadResAndShow"),
            222 => (Self::ClassifyView, 28, "mmo.petpackage.controller.skin", "PackageSkinManager", "changeClassify"),
            224 => (Self::DisposeSkin, 28, "mmo.petpackage.controller.skin", "PackageSkinManager", "dispose"),
            317 => (Self::ConstructPanel, 41, "mmo.petpackage.panel", "PetPackagePanel", ""),
            375 => (Self::DisposePanel, 41, "mmo.petpackage.panel", "PetPackagePanel", "disposeEvent"),
            1231 => (Self::ClickView, 128, "mmo.petpackage.panel", "SimpleViewController", "onClickMe"),
            1232 => (Self::ChangeStyle, 128, "mmo.petpackage.panel", "SimpleViewController", "changeStyle"),
            1646 if crate::pet_switch_frame::enabled() => (Self::SwitchPet, 143, "mmo.petfightnew.common.fightcontroller", "SwitchPm", "switchPet"),
            4023 => (Self::ConstructFight, 720, "mmo.petfightnew.common", "FightPanel", ""),
            4073 => (Self::DisposeFightManagers, 720, "mmo.petfightnew.common", "FightPanel", "disposeManagers"),
            4167 => (Self::DisposeFight, 720, "mmo.petfightnew.common", "FightPanel", "dispose"),
            _ => return Self::Other,
        };
        static ENABLED: OnceLock<bool> = OnceLock::new();
        let sampling = *ENABLED.get_or_init(|| std::env::var_os("AOLA_SWF_OPERATIONS").is_some_and(|v| v == "1"));
        let battle = matches!(operation, Self::ConstructFight | Self::DisposeFightManagers | Self::DisposeFight);
        if !(matches!(operation, Self::SwitchPet | Self::OpenActivityPanel) || sampling || (battle && (ruffle_render::backend::aola_long_run_maintenance()
            || crate::goto_profile::goto_target_only_mode() == "battle"
            || crate::orphan_manager::ordinary_orphan_mode() == "battle"))) {
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
        } else if matches!(operation, Self::ConstructPanel | Self::ConstructFight) {
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
    #[ignore = "Set AOLA_SWF_OPERATIONS=1 and AOLA_TEST_BATTLE_FILE to an actual petfightnew SWF"]
    fn battle_operations_bind_to_real_abc() {
        use swf::avm2::read::Reader;
        use swf::avm2::types::{Index, Multiname, Op, TraitKind};
        use swf::extensions::ReadSwfExt;

        let path = std::env::var_os("AOLA_TEST_BATTLE_FILE").unwrap();
        let data = std::fs::read(path).unwrap();
        let buffer = swf::decompress_swf(data.as_slice()).unwrap();
        let movie = swf::parse_swf(&buffer).unwrap();
        let mut abc = movie.tags.iter().find_map(|tag| {
            let swf::Tag::DoAbc2(tag) = tag else { return None };
            let abc = Reader::new(tag.data).read().unwrap();
            (Operation::for_method(&abc, 4023) == ConstructFight).then_some(abc)
        }).expect("FightPanel constructor");
        assert_eq!(Operation::for_method(&abc, 4073), DisposeFightManagers);
        assert_eq!(Operation::for_method(&abc, 4167), DisposeFight);
        assert_eq!(Operation::for_method(&abc, 4168), Other);

        let body = &abc.method_bodies[abc.methods[4167].body.unwrap().0 as usize];
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
        abc.instances[720].init_method = Index::new(0);
        assert_eq!(Operation::for_method(&abc, 4023), Other);
        abc.instances[720].traits.retain(|t| !matches!(t.kind, TraitKind::Method { method, .. } if method.0 == 4073));
        assert_eq!(Operation::for_method(&abc, 4073), Other);
        abc.instances[720].name = Index::new(0);
        assert_eq!(Operation::for_method(&abc, 4167), Other);
    }
}
