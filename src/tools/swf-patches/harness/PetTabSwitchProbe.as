package {
    import flash.display.Sprite;
    import flash.events.Event;
    import flash.utils.Dictionary;
    import flash.utils.getDefinitionByName;
    import probe.Paint;
    import mmo.petpackage.controller.event.PetPackageEvent;
    import mmo.petpackage.controller.event.PetPackageEventDispatcher;
    import mmo.petpackage.data.PetDetailDataMap;
    public class PetTabSwitchProbe extends Sprite {
        private var owners:Dictionary = new Dictionary();
        private var frames:int;
        public function PetTabSwitchProbe() {
            for each (var page:String in ["PetStudyInfo", "PetDetailAbility", "PetItem", "GodCard", "SimpleViewController"]) {
                var path:String = "mmo.petpackage.panel." + (page == "SimpleViewController" ? "" : "inclusion.petdetailinfo.");
                var type:Class = getDefinitionByName(path + page) as Class;
                var view:Object = new type();
                PetDetailDataMap.reads = Paint.inits = Paint.paints = 0;
                Paint.latest = "-";
                send(7, 0);
                if (PetDetailDataMap.reads != 1 || Paint.inits != 0 || Paint.paints != 0)
                    throw new Error("pre-tab initialization " + page);
                PetDetailDataMap.reads = 0;
                view.removeUI();
                send(10, 1); send(20, 2);
                var hidden:int = PetDetailDataMap.reads;
                view.updateUI();
                // Complete initial loading after another switch away and two updates.
                view.removeUI();
                send(30, 3); send(0, 4);
                view.setCanUpdate();
                var late:int = Paint.paints;
                view.removeUI();
                view.updateUI();
                var shown:String = Paint.latest;
                send(0, 5); // same id with new data while active
                view.updateUI(); // reselect should use this snapshot
                var visible:String = Paint.latest;
                view.removeUI();
                send(-1, 6);
                view.updateUI();
                var negative:String = Paint.latest;
                view.removeUI();
                send(404, 7); // cache miss must clear the prior pdi on return
                view.updateUI();
                view.setCanUpdate();
                var nullPaints:int = Paint.paints;
                send(99, 8);
                view.removeUI();
                send(100, 9);
                var paints:int = Paint.paints;
                view.dispose();
                send(9, 10);
                view.setCanUpdate();
                trace(page, hidden, late, shown, visible, negative, Paint.inits,
                      nullPaints, paints, Paint.paints, PetDetailDataMap.reads);
            }
            owners["aoqiUseDisplayOwners"]();
            exercisePair();
            addEventListener(Event.ENTER_FRAME, checkRelease);
        }
        private function exercisePair():void {
            var type:Class = getDefinitionByName("mmo.petpackage.panel.inclusion.petdetailinfo.PetStudyInfo") as Class;
            var first:Object = new type(), second:Object = new type();
            var a:Function = first.onPetDetailedCached, b:Function = second.onPetDetailedCached;
            a.aolaIdentityProbe = 17;
            if (a !== first.onPetDetailedCached || a === b || first.onPetDetailedCached.aolaIdentityProbe != 17 ||
                b.aolaIdentityProbe !== undefined) throw new Error("bound listener identity or per-instance storage");
            delete a.aolaIdentityProbe;
            PetDetailDataMap.reads = Paint.paints = 0;
            send(5, 1); // both instances are still active prior to the first removeUI
            if (PetDetailDataMap.reads != 2 || Paint.paints != 0) throw new Error("initial dispatch");
            first.removeUI();
            send(6, 2);
            var before:int = PetDetailDataMap.reads;
            second.updateUI(); second.setCanUpdate();
            first.updateUI(); first.setCanUpdate();
            first.dispose(); second.dispose();
            if (PetPackageEventDispatcher.instance.hasEventListener(PetPackageEvent.PET_DETAIL_UPDATE) ||
                PetPackageEventDispatcher.instance.hasEventListener(PetPackageEvent.ONLY_UPDATE_PANEL))
                throw new Error("listener still registered");
            owners[first] = true; owners[second] = true;
            trace("pair", before, PetDetailDataMap.reads, Paint.paints, Paint.latest, countOwners());
        }
        private function countOwners():int {
            var count:int = 0;
            for (var key:* in owners) count++;
            return count;
        }
        private function checkRelease(e:Event):void {
            for (var i:int = 0; i < 1200; i++) { var temp:Object = {i:i}; }
            if (++frames == 120) {
                removeEventListener(Event.ENTER_FRAME, checkRelease);
                trace("disposed remaining", countOwners());
            }
        }
        private function send(id:int, version:int):void {
            PetDetailDataMap.version = version;
            PetPackageEventDispatcher.instance.dispatchEvent(new PetPackageEvent(PetPackageEvent.PET_DETAIL_UPDATE, {id:id}));
        }
    }
}
package probe {
    import mmo.interfaces.pet.data.PetDetailedInfo;
    public class Paint {
        public static var inits:int, paints:int, latest:String;
        public static function initUI(value:PetDetailedInfo):void { inits++; }
        public static function refreshUI(value:PetDetailedInfo):void {
            paints++; latest = value.id + "/" + value.version;
        }
    }
}
package mmo.petpackage.controller.event {
    import flash.events.Event;
    public class PetPackageEvent extends Event {
        public static const PET_DETAIL_UPDATE:String = "detail", ONLY_UPDATE_PANEL:String = "only";
        public var params:Object;
        public function PetPackageEvent(name:String, data:Object) { super(name); params = data; }
    }
}
package mmo.petpackage.controller.event {
    import flash.events.EventDispatcher;
    public class PetPackageEventDispatcher extends EventDispatcher {
        public static var instance:PetPackageEventDispatcher = new PetPackageEventDispatcher();
    }
}
package mmo.petpackage.data {
    import mmo.interfaces.pet.data.PetDetailedInfo;
    public class PetDetailDataMap {
        public static var instance:PetDetailDataMap = new PetDetailDataMap();
        public static var reads:int, version:int;
        public function getPetDetailInfo(id:int):PetDetailedInfo {
            reads++;
            if (id == 404) return null;
            var detail:PetDetailedInfo = new PetDetailedInfo(); detail.id = id; detail.version = version;
            return detail;
        }
    }
}
package mmo.interfaces.pet.data {
    public class PetDetailedInfo { public var id:int, version:int; }
}
package mmo.petpackage.controller.helper {
    public class ShowWantChangePanelHelper { public static function setButtonState(view:Object):void {} }
}
package mmo.common.act_operation { public class PetFunctionShortcut {} }
