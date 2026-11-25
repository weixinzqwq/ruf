package {
    import flash.display.Sprite;
    import flash.utils.getDefinitionByName;
    import mmo.petpackage.controller.event.*;
    import mmo.interfaces.uicomponent.mutex.MutexAry;
    import probe.Paint;

    public class PetActiveTabProbe extends Sprite {
        public function PetActiveTabProbe() { run(true); run(false); }
        private function run(panelFirst:Boolean):void {
            var panelType:Class = getDefinitionByName("mmo.petpackage.panel.PetPackagePanel") as Class;
            var skillType:Class = getDefinitionByName("mmo.petpackage.panel.inclusion.petdetailinfo.PetDetailSkill") as Class;
            var panel:Object = new panelType();
            var bus:PetPackageEventDispatcher = PetPackageEventDispatcher.instance = new PetPackageEventDispatcher();
            if (panelFirst) bus.addEventListener("detail", panel.onCurrentPetUpdate);
            var skill:Object = new skillType();
            if (!panelFirst) bus.addEventListener("detail", panel.onCurrentPetUpdate);
            var row:MutexAry = new MutexAry();
            panel.mcskill = skill;
            panel.mutexTabGroup0 = new MutexAry();
            panel.mutexTabGroup1Ary = [row];
            panel.mutexInfos = [[skill]];
            Paint.inits = 0; Paint.ids = [];
            panel.setTabInfoVisible(); // ordinary init before the detail or service is ready
            send(1); skill.setCanUpdate(); // exercise the actual completion method
            if (Paint.inits != 2 || Paint.ids.join() != "1") throw new Error("deferred initialization");
            Paint.ids = [];
            send(2);
            var active:String = Paint.ids.join();
            Paint.ids = [];
            panel.setTabInfoVisible(); // explicit reselection must refresh
            row.index = 1; panel.setTabInfoVisible(); // hide
            if (skill.visible) throw new Error("hide failed");
            send(3); // the skill's own listener still receives hidden events
            row.index = 0; send(4); // canonical event also reopens it
            if (!skill.visible) throw new Error("reopen failed");
            trace(panelFirst ? "panel-first" : "page-first", active, Paint.ids.join());
        }
        private function send(id:int):void {
            PetPackageEventDispatcher.instance.dispatchEvent(new PetPackageEvent("detail", {id:id}));
        }
    }
}
package probe {
    import mmo.interfaces.pet.data.PetDetailedInfo;
    public class Paint {
        public static var inits:int;
        public static var ids:Array;
        public static function initUI(info:PetDetailedInfo):void { inits++; }
        public static function refreshUI(info:PetDetailedInfo):void { ids.push(info.id); }
    }
}
package mmo.interfaces.uicomponent.mutex {
    public class MutexAry { public var index:int; public function getSelectedIndex():int { return index; } }
}
package mmo.petpackage.controller.event {
    import flash.events.Event;
    public class PetPackageEvent extends Event {
        public static const PET_DETAIL_UPDATE:String = "detail", ONLY_UPDATE_PANEL:String = "only";
        public var params:Object;
        public function PetPackageEvent(type:String, data:Object) { super(type); params = data; }
    }
}
package mmo.petpackage.controller.event {
    import flash.events.EventDispatcher;
    public class PetPackageEventDispatcher extends EventDispatcher { public static var instance:PetPackageEventDispatcher; }
}
package mmo.petpackage.data {
    import mmo.interfaces.pet.data.PetDetailedInfo;
    public class PetDetailDataMap {
        public static var instance:PetDetailDataMap = new PetDetailDataMap();
        public function getPetDetailInfo(id:int):PetDetailedInfo {
            var p:PetDetailedInfo = new PetDetailedInfo(); p.id = id; return p;
        }
    }
}
package mmo.interfaces.pet.data { public class PetDetailedInfo { public var id:int; } }
package mmo.petpackage.controller.helper {
    public class ShowWantChangePanelHelper { public static function setButtonState(view:Object):void {} }
}
package mmo.common.act_operation { public class PetFunctionShortcut {} }
