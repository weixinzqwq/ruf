package {
    import flash.display.Sprite;
    import flash.system.ApplicationDomain;
    import flash.utils.getDefinitionByName;
    import mmo.interfaces.callback.FunCallInfo;
    import mmo.interfaces.pet.data.PetBriefInfo;
    import mmo.loader.scheduler.ResScheduler;
    import mmo.petfightnew.common.FightData;
    import mmo.petfightnew.common.FightPanel;
    import mmo.petfightnew.common.MCPetContainer;
    import mmo.petfightnew.common.manager.FightTweenMaxManager;
    import probe.PortraitHost;

    public class TransformPortraitProbe extends Sprite {
        public function TransformPortraitProbe() {
            var methods:Array = [
                ["yiren", "showYiRenMovie", "yiren", "yirenfirstappeareffect", "mmo.yiren.effect.YiRenFirstAppearMovie"],
                ["jiuri", "showJiuRiShenAppearMovie", "jiurichongxian", "jiurichongxianeffect_", "mmo.gqpet.jiurichongxian.effect.JiuRiChongXianAppearEffect_"],
                ["juexing", "showJuexingAppearMovie", "juexing", "juexingeffect_", "mmo.gqpet.juexing.effect.JuexingAppearEffect_"],
                ["dengshen", "showDengShenChangJieAppearMovie", "dengshenchangjie", "dengshenchangjieeffect_", "mmo.gqpet.dengshenchangjie.effect.GQDengShenChangJieAppearEffect_"],
                ["zhenli", "showZhenLiFaDianAppearMovie", "zhenlifadian", "zhenlifadianeffect_", "mmo.gqpet.zhenlifadian.effect.GQZhenLiFaDianAppearEffect_"]
            ];
            try {
                for each (var row:Array in methods) {
                    for each (var material:int in [row[0] == "yiren" ? 5225 : 5625, 6371]) {
                        for each (var enabled:Boolean in [false, true])
                            exercise(row, material, enabled, false);
                    }
                    exercise(row, 6371, true, false, "top");
                    exercise(row, 6371, true, false, "only");
                    trace(row[0], "6 cases passed");
                }
                exercise(methods[0], 5225, true, true);
                exercise(methods[0], 5225, false, true);
                trace("repeat yiren 2 cases passed");
            } catch (e:Error) {
                trace("FAILED", e, e.getStackTrace());
            }
        }

        private function check(value:Boolean, description:String):void {
            if (!value) throw new Error(description);
        }

        private function exercise(row:Array, material:int, enabled:Boolean, repeat:Boolean, depth:String = "middle"):void {
            var viewType:Class = getDefinitionByName("mmo.petfightnew.common.fightcontroller.ChangePetView") as Class;
            var view:Object = new viewType();
            var panel:FightPanel = new FightPanel();
            var before:MCPetContainer = new MCPetContainer();
            var after:MCPetContainer = new MCPetContainer();
            before.x = 113;
            before.y = 57;
            panel.petMCList[1] = before;
            var under:Sprite = new Sprite();
            var over:Sprite = new Sprite();
            if (depth != "only") panel.layerPet.addChild(under);
            panel.layerPet.addChild(before);
            if (depth == "middle") panel.layerPet.addChild(over);
            view.init(panel);
            view._petContainer = after;
            view._locaId = 1;
            view._isFace = material == 6371;
            var pet:PetBriefInfo = new PetBriefInfo();
            pet.id = 27;
            pet.raceId = 10000 + material;
            pet.viewId = material;
            var key:String = "gqYirenChangeView27_1";
            FightData.instance.extData = {};
            if (repeat) FightData.instance.extData[key] = true;
            FightTweenMaxManager.instance.created = 0;
            PortraitHost.enabled = enabled;
            PortraitHost.calls = [];
            ResScheduler.requests = [];
            var completed:int = 0;
            var isYiren:Boolean = row[0] == "yiren";
            var usesDefaultPosition:Boolean = row[0] == "dengshen" || row[0] == "zhenli";
            var context:String = row[0] + "/" + material + "/" + enabled + "/repeat=" + repeat + "/" + depth;
            var completion:FunCallInfo = new FunCallInfo(function():void {
                completed++;
                check(before.disposed == 1 && before.parent == null, "previous model still active " + context);
                check(after.parent == panel.layerPet && panel.petMCList[1] === after, "model not installed at callback " + context);
                check(after.x == (usesDefaultPosition ? 211 : 113) && after.y == (usesDefaultPosition ? 64 : 57),
                      "model position lost " + context);
                var expectedCount:int = depth == "only" ? 1 : depth == "top" ? 2 : 3;
                check(panel.layerPet.numChildren == expectedCount && panel.layerPet.getChildIndex(after) == expectedCount - 1,
                      "model at wrong depth " + context);
                if (depth != "only") check(panel.layerPet.getChildAt(0) === under, "reordered lower layer " + context);
                if (depth == "middle") check(panel.layerPet.getChildAt(1) === over, "reordered upper layer " + context);
                check(!isYiren || FightData.instance.extData[key] === true, "first flag absent at callback " + context);
            });
            view[row[1]](ApplicationDomain.currentDomain, 1, pet, completion);
            if (!enabled || repeat) {
                check(completed == 0 && before.parent == panel.layerPet, "expected queued resource " + context);
                check(ResScheduler.requests.length == 1, "expected one request " + context);
                var resource:Object = ResScheduler.requests[0];
                var basename:String = repeat ? "yirenappeareffect" : row[3];
                var effectClass:String = repeat ? "mmo.yiren.effect.YiRenAppearMovie" : row[4];
                var expectedClass:String = effectClass + material + (isYiren ? "" : (view._isFace ? "_0" : "_1"));
                check(resource.path == "gqpet/" + row[2] + "/effect/" + basename + material, "resource " + resource.path + " " + context);
                check(resource.className == expectedClass, "resource class " + resource.className + " " + context);
                // Deliver the official error completion later, after the request method has returned.
                resource.failed(null);
            } else {
                check(ResScheduler.requests.length == 0, "unexpected portrait load " + context);
            }
            check(completed == 1, "completion count " + completed + " " + context);
            check(FightTweenMaxManager.instance.created == (isYiren ? 1 : 0), "unexpected model tween " + context);
            check(PortraitHost.calls.join(",") == (repeat ? "" : "transform-portrait-" + row[0]), "host call " + context);
        }
    }
}
package probe {
    public class PortraitHost {
        public static var enabled:Boolean;
        public static var calls:Array;
        public static function call(name:String, channel:String):Boolean {
            if (name != "aolaBlockSkill") throw new Error("Unexpected host interface " + name);
            calls.push(channel);
            return enabled;
        }
    }
}
package mmo.petfightnew.common {
    import flash.display.Sprite;
    public class MCPetContainer extends Sprite {
        public var disposed:int;
        public function dispose():void { disposed++; }
    }
}
package mmo.petfightnew.common {
    import flash.display.Sprite;
    public class FightPanel {
        public var petMCList:Array = [];
        public var layerPet:Sprite = new Sprite();
    }
}
package mmo.petfightnew.common {
    import flash.system.ApplicationDomain;
    public class FightData {
        public static var instance:FightData = new FightData();
        public var extData:Object;
        public var applicationDomain:ApplicationDomain = ApplicationDomain.currentDomain;
    }
}
package mmo.common {
    import flash.display.DisplayObject;
    public class DisplayUtil {
        public static function stopAndRemove(child:DisplayObject):void {
            if (child.parent != null) child.parent.removeChild(child);
        }
    }
}
package mmo.interfaces.gq {
    public class GQPetType {
        public static function getYiRenObject(id:int):Object { return {viewId:id}; }
    }
}
package mmo.petfightnew.common.util {
    import flash.geom.Point;
    public class PositionUtil {
        public static function getDefaultPosition(slot:int):Point {
            if (slot != 1) throw new Error("Wrong model slot");
            return new Point(211, 64);
        }
    }
}
package mmo.interfaces.pet.data {
    public class PetAssetsMap { public static function getPetFlaId(id:int):int { return id % 10000; } }
}
package mmo.interfaces.pet.data {
    public class PetBriefInfo { public var id:int; public var raceId:int; public var viewId:int; }
}
package mmo.interfaces.callback {
    public class FunCallInfo {
        private var done:Function;
        public function FunCallInfo(action:Function) { done = action; }
        public function invoke():void { done(); }
    }
}
package mmo.loader.scheduler {
    public class ResScheduler {
        public static var requests:Array;
        public static function getRes(resource:Object):void { requests.push(resource); }
    }
}
package mmo.loader.scheduler {
    public class ResParams {
        public static function createTempByDomain(path:String, success:Function, domain:*, className:String, priority:*, failed:Function):Object {
            return {path:path, className:className, failed:failed};
        }
    }
}
package mmo.loader.scheduler {
    public class ResDomain { public static function decorate(domain:*):* { return domain; } }
}
package mmo.loader.scheduler { public class ResPriority { public static const NORMAL:int = 1; } }
package com.greensock.data {
    public class TweenLiteVars {
        public function prop(name:String, value:*):void {}
        public function useFrames(value:Boolean):void {}
        public function onComplete(callback:Function):void {}
    }
}
package com.greensock {
    public class TweenMax { public function TweenMax(target:*, duration:Number, options:*) {} }
}
package mmo.petfightnew.common.manager {
    public class FightTweenMaxManager {
        public static var instance:FightTweenMaxManager = new FightTweenMaxManager();
        public var created:int;
        public function addTweenMax(tween:*):void { created++; }
    }
}
package mmo.petfightnew.common.manager {
    public class ServicePool {
        public static var instance:ServicePool = new ServicePool();
        public function getService(type:Class):Object { return null; }
    }
}
package mmo.interfaces.pet.pm.services { public interface IPMService {} }
