//! Observable method returns in the 2026111953323155 backpack and its same-version controls.
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
}

impl Operation {
    pub(super) fn for_method(abc: &AbcFile, index: u32) -> Self {
        // Check the cheap method index first. Only six possible indices need further work.
        let (operation, instance, package, class, member) = match index {
            222 => (Self::ClassifyView, 28, "mmo.petpackage.controller.skin", "PackageSkinManager", "changeClassify"),
            224 => (Self::DisposeSkin, 28, "mmo.petpackage.controller.skin", "PackageSkinManager", "dispose"),
            317 => (Self::ConstructPanel, 41, "mmo.petpackage.panel", "PetPackagePanel", ""),
            375 => (Self::DisposePanel, 41, "mmo.petpackage.panel", "PetPackagePanel", "disposeEvent"),
            1231 => (Self::ClickView, 128, "mmo.petpackage.panel", "SimpleViewController", "onClickMe"),
            1232 => (Self::ChangeStyle, 128, "mmo.petpackage.panel", "SimpleViewController", "changeStyle"),
            _ => return Self::Other,
        };
        static ENABLED: OnceLock<bool> = OnceLock::new();
        if !ENABLED.get_or_init(|| std::env::var_os("AOLA_SWF_OPERATIONS").is_some_and(|v| v == "1")) {
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
        let bound = if operation == Self::ConstructPanel {
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
}
