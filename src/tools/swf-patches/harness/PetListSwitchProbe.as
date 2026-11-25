package {
    import flash.display.MovieClip;
    import flash.display.Sprite;
    import flash.utils.getDefinitionByName;
    import mmo.interfaces.pet.data.PetBriefInfo;
    import mmo.petpackage.data.PetDetailDataMap;
    import mmo.petpackage.panel.inclusion.petlist.PetSingleIconDetail;
    import probe.*;

    public class PetListSwitchProbe extends Sprite {
        private function check(ok:Boolean, message:String):void { if (!ok) throw new Error(message); }

        public function PetListSwitchProbe() {
            try { exercise(); } catch (e:Error) { trace("FAILED", e, e.getStackTrace()); }
        }

        private function exercise():void {
            var prefix:String = "mmo.petpackage.panel.inclusion.";
            var listType:Class = getDefinitionByName(prefix + "PetList") as Class;
            var slotType:Class = getDefinitionByName(prefix + "petlist.PetSingleIconBase") as Class;
            var list:Object = new listType();
            list.pets = [];
            list._objs = [];
            list.tempPBI = new PetBriefInfo();
            list.ipds = new PetService();
            var full:MovieClip = new MovieClip();
            list._mcPetIconFullScreen = full;
            for (var i:int = 0; i < 6; i++) {
                var slot:Object = new slotType();
                slot.mcHp = new MeasuredClip();
                slot.mcNormal = new MeasuredClip();
                slot.mcSelect = new MeasuredClip();
                slot.mcSelect.visible = false;
                slot.mcAdd = new MovieClip();
                slot.mcPetLabel = new MovieClip();
                slot._mcDetail = new PetSingleIconDetail();
                list.pets.push(slot);
                list._objs.push({pmSView:{i:i + 1}, hp:50, race:55, title:"name"+i});
                var icon:MovieClip = new MovieClip();
                icon.name = "mcPet_" + i;
                full.addChild(icon);
            }
            list.onPMDetailUpdate(null);
            list.setSelectedPet(list.pets[0]);
            list.setSelectedPet(list.pets[1]);
            for each (slot in list.pets) {
                check(slot.mcHp.currentFrame == 25 && slot.mcNormal.currentFrame == 5, "first render");
                slot.mcHp.play();
            }
            MeasuredClip.gotos = 0;
            list.onPMDetailUpdate(null);
            trace("unchanged slot gotos", MeasuredClip.gotos);
            check(!list.pets[0].selected && list.pets[1].selected, "prior and current selection");
            check(full.getChildAt(0)["mark"] == 1 && full.getChildAt(1)["mark"] == 2, "full screen selection");
            for each (slot in list.pets) check(!slot.mcHp.isPlaying, "same-frame stop after play");

            // An unselected slot changes in place while the selected slot gets
            // independently updated cached detail, with unchanged brief identity.
            list._objs[4].hp = 20;
            list._objs[4].race = 1;
            list._objs[4].title = "renamed";
            var detail:PetBriefInfo = new PetBriefInfo();
            detail.parseInfo(list._objs[1]);
            detail.curHp = 10;
            PetDetailDataMap.instance.cache[2] = detail;
            list.onPMDetailUpdate(null);
            check(list.pets[4].mcHp.currentFrame == 10 && list.pets[4].mcNormal.currentFrame == 1, "third-party slot");
            check(list.pets[4].mcPetLabel.race == 1 && full.getChildAt(4)["txtPetName"] == "renamed", "secondary displays");
            check(list.pets[1].mcHp.currentFrame == 5 && list.pets[1]._mcDetail.hp == 10, "same-ID cache update");
            list._objs.reverse();
            list._objs.length = 5;
            list.onPMDetailUpdate(null);
            check(list.pets[0].id == 6 && list.pets[1].id == 5, "reordered entries");
            check(list.pets[5].id == -1 && list.pets[5].mcAdd.visible && !list.pets[5].mcHp.visible
                  && list.pets[5].mcNormal.currentFrame == 1 && !full.getChildAt(5).visible, "empty slot");
            list._objs.push({pmSView:{i:9}, hp:60, race:55, title:"refilled"});
            list.onPMDetailUpdate(null);
            check(list.pets[5].id == 9 && list.pets[5].mcHp.currentFrame == 30 && !list.pets[5].mcAdd.visible
                  && full.getChildAt(5).visible && full.getChildAt(5)["txtPetName"] == "refilled", "refill");
            trace("selection hp reorder empty fullscreen passed");
        }
    }
}
package probe {
    import flash.display.MovieClip;
    public class MeasuredClip extends MovieClip {
        public static var gotos:int;
        override public function gotoAndStop(frame:Object, scene:String = null):void {
            gotos++;
            super.gotoAndStop(frame, scene);
        }
    }
}
package mmo.petpackage.controller.event { public class PetPackageEvent {} }
package mmo.interfaces.pet.data { public interface IPetDataService { function setSelectPetId(id:int):void; } }
package probe {
    import mmo.interfaces.pet.data.IPetDataService;
    public class PetService implements IPetDataService { public function setSelectPetId(id:int):void {} }
}
package mmo.interfaces.pet.data {
    public class PetBriefInfo {
        public var id:int, curHp:int, raceId:int, maxHp:int = 100;
        public var name:String, isGetAoYiSkill:Boolean;
        public function parseInfo(raw:Object):void {
            id = raw.pmSView.i; curHp = raw.hp; raceId = raw.race; name = raw.title;
        }
    }
}
package mmo.petpackage.data {
    import mmo.interfaces.pet.data.PetBriefInfo;
    public class PetDetailDataMap {
        public static var instance:PetDetailDataMap = new PetDetailDataMap();
        public var cache:Object = {};
        public function checkCache(id:int):Boolean { return cache[id] != null; }
        public function getPetDetailInfo(id:int):PetBriefInfo { return cache[id]; }
        public function saveCurrentPet(id:int):void {}
        public function tryUpdateCachhe(id:int):void {}
    }
}
package mmo.petpackage.panel.inclusion.petlist {
    import flash.display.MovieClip;
    import mmo.interfaces.pet.data.PetBriefInfo;
    public class PetSingleIconDetail extends MovieClip {
        public var hp:int;
        public function setData(raw:Object, data:PetBriefInfo):void { hp = data.curHp; }
    }
}
package mmo.interfaces.pet.pm.services { public interface IPMService { function get customData():Object; } }
package mmo.interfaces {
    import probe.ModelService;
    public class ServiceContainer { public static function getService(id:Object):Object { return new ModelService(); } }
}
package probe {
    import mmo.interfaces.pet.pm.services.IPMService;
    import mmo.interfaces.pet.pm.data.custom.PMData;
    public class ModelService implements IPMService {
        public function get customData():Object { return {pmDataMap:this}; }
        public function getPM(race:int):PMData { return new PMData(race); }
    }
}
package mmo.interfaces.pet.pm.data.custom {
    public class PMData {
        private var race:int;
        public function PMData(id:int) { race = id; }
        public function isXinghuiPM():Boolean { return false; }
        public function isDegeneratorPM():Boolean { return false; }
        public function isLegendPM():Boolean { return race == 55; }
        public function isCompletePM():Boolean { return false; }
        public function isMiraclePet():Boolean { return false; }
    }
}
package mmo.play.actthings.helper {
    import flash.display.MovieClip;
    public class ActComHelper {
        public static function getChildNode(root:Object, key:String):MovieClip { return root[key]; }
        public static function setTxtValue(clip:MovieClip, key:String, value:String):void { clip[key] = value; }
        public static function setMcFrame(root:MovieClip, key:String, frame:int):void { root.getChildByName(key)["mark"] = frame; }
    }
}
package mmo.play.pet.petlabel {
    import flash.display.MovieClip;
    public class PetLabelIconHelper {
        public static function clearLabelIcon(clip:MovieClip):void { clip.race = null; }
        public static function loadIconByRaceId(id:int, clip:MovieClip, kind:int, scale:Number):void { clip.race = id; }
    }
}
package mmo.play.pet.petlabel { public class PetLabelConfig { public static const LevelType_1:int = 1; } }
