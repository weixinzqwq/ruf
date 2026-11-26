package {
    import flash.display.Sprite;
    import flash.events.Event;
    import probe.external.ExternalInterface;
    import probe.GameSites;
    import mmo.interfaces.pet.data.PetAssetsMap;
    import mmo.loader.scheduler.ResScheduler;
    import probe.ProbeConfig;

    public class MyProModelProbe extends Sprite {
        private var sites:GameSites = new GameSites();
        private var frame:int;
        public function MyProModelProbe() {
            ExternalInterface.target = 29;
            sites.isFace = true;
            trace("body", sites.body(), ExternalInterface.last, sites.flaId);
            sites.isFace = false;
            trace("missing side", sites.body(), ExternalInterface.last, sites.flaId);
            trace("icon", sites.icon(7), ExternalInterface.last, PetAssetsMap.calls);
            ExternalInterface.target = 87;
            trace("missing target", sites.icon(7), ExternalInterface.last);
            ExternalInterface.target = 29;
            ExternalInterface.accept = false;
            sites.isFace = true;
            trace("stale rule", sites.body(), ExternalInterface.last);
            ExternalInterface.available = false;
            trace("no host", sites.body(), sites.icon(7), PetAssetsMap.calls);
            ExternalInterface.available = true;
            ExternalInterface.accept = true;
            ExternalInterface.target = 31;
            trace("bitmap", sites.body(), sites.body(), ProbeConfig.reads);
            ExternalInterface.target = 32;
            trace("partial bitmap", sites.body());
            ExternalInterface.target = 47;
            ExternalInterface.binding = 1;
            trace("cold", sites.body(), sites.body(), ResScheduler.urls.length);
            addEventListener(Event.ENTER_FRAME,advance);
        }
        private function advance(e:Event):void {
            if (++frame == 4) {
                trace("prepared", sites.body(), ResScheduler.urls.join(","));
                ExternalInterface.target = 48;
                trace("queued stale", sites.body(), sites.icon(7));
                ExternalInterface.binding++;
            }
            if (frame == 10) {
                trace("stale requests", ResScheduler.urls.length);
                sites.isFace = false;
                ExternalInterface.target = 6044;
                trace("split cold", sites.body(), ExternalInterface.reason);
            }
            if (frame == 17) {
                trace("split ready", sites.body(), ResScheduler.urls.slice(1), ExternalInterface.progressEvents.slice(-4));
                ++ExternalInterface.binding;
                ResScheduler.urls = ["fightassets/bitmap/pet6044/pet6044_1_0"];
                sites.isFace = true;
                trace("partially warm", sites.body(), ExternalInterface.reason);
            }
            if (frame == 24) {
                trace("completed partial", sites.body(), ResScheduler.urls);
                removeEventListener(Event.ENTER_FRAME,advance);
            }
        }
    }
}
package probe {
    public class GameSites {
        public var flaId:int = 17;
        public var isFace:Boolean;
        public function body():int { return -1; }
        public function icon(id:int):int { return -1; }
    }
}
package probe.external {
    public class ExternalInterface {
        public static var available:Boolean = true;
        public static var target:int;
        public static var binding:int;
        public static var accept:Boolean = true;
        public static var last:String;
        public static var reason:String;
        public static var progressEvents:Array = [];
        public static function call(name:String,...args):* {
            if (name == "aolaMyProBinding") return binding;
            if (name == "aolaMyProModelCandidate") return target;
            if (name == "aolaMyProModelUse") {
                last = args.slice(0,5).join(",");
                reason = args[5];
                return accept && args[4] ? args[2] : args[1];
            }
            if (name == "aolaMyProModelProgress") progressEvents.push(args.slice(5).join("/"));
            return null;
        }
    }
}
package mmo.interfaces.pet.data {
    public class PetAssetsMap {
        public static var calls:int;
        public static function getPetFlaId(id:int):int { ++calls; return id+10; }
    }
}
package mmo.petfightnew.common {
    import flash.system.ApplicationDomain;
    public class FightData {
        public static var instance:FightData = new FightData();
        private static var view:Object = {applicationDomain:ApplicationDomain.currentDomain};
        public function getPetViewDomain():Object { return view; }
        public function getPetMovieStrategyType(id:int):int { return id == 31 || id == 32 || id == 6044 ? 2 : 1; }
    }
}
package mmo.play.petfight.common {
    public class PetMovieStrategyType { public static const VECTORGRAPH:int = 1; }
}
package mmo.play.petfight.common.config {
    public class FightResConfig {
        public static function getSinglePetMovieClsName(id:int,side:int):String {
            if (id == 47 && mmo.loader.scheduler.ResScheduler.urls.length) id = 29;
            return "probe.Model" + id + "_" + side;
        }
        public static const FILE_NUM_OF_COMPLEMENTARY_PET_MOVIE:int = 3;
        public static function isComplementaryPetMovie(id:int, side:int):Boolean { return id >= 4400; }
        public static function getFilePathOfComplementaryPetMovie(id:int,side:int,part:int):String {
            return "fightassets/bitmap/pet"+id+"/pet"+id+"_"+side+"_"+part;
        }
        public static function getPetBitmapMovieConfigClsName(id:int,side:int):String {
            if(id == 6044) {
                var urls:Array = mmo.loader.scheduler.ResScheduler.urls;
                if(urls.indexOf(getFilePathOfComplementaryPetMovie(id,side,0)) < 0) return "probe.NotLoaded";
                if(urls.indexOf(getFilePathOfComplementaryPetMovie(id,side,2)) < 0) return "probe.PartialConfig";
            }
            return "probe.BitmapConfig";
        }
        public static function getPetMainFilePath(strategy:int,id:int,side:int):String {
            return isComplementaryPetMovie(id,side) ? getFilePathOfComplementaryPetMovie(id,side,0) : "mock/" + id;
        }
    }
}
package probe { import flash.display.MovieClip; public class Model29_1 extends MovieClip {} }
package probe {
    import flash.display.MovieClip;
    public class BitmapConfig extends MovieClip { public var MovieConfig:Class = ProbeConfig; }
}
package probe {
    import flash.utils.ByteArray;
    public class ProbeConfig extends ByteArray {
        public static var reads:int;
        public function ProbeConfig() { ++reads; writeUTFBytes("1:0,0,0;1,0,0"); }
    }
}
package probe {
    import flash.display.MovieClip;
    public class PartialConfig extends MovieClip { public var MovieConfig:Class = PartialBytes; }
}
package probe {
    import flash.utils.ByteArray;
    public class PartialBytes extends ByteArray { public function PartialBytes() { writeUTFBytes("1:9,0,0"); } }
}
package mmo.petfight.bitmap {
    import flash.display.BitmapData;
    public class pet6044_2_1_0 extends BitmapData { public function pet6044_2_1_0(){super(2,2);} }
}
package mmo.petfight.bitmap {
    import flash.display.BitmapData;
    public class pet6044_2_1_1 extends BitmapData { public function pet6044_2_1_1(){super(2,2);} }
}
package mmo.petfight.bitmap {
    import flash.display.BitmapData;
    public class pet6044_1_1_0 extends BitmapData { public function pet6044_1_1_0(){super(2,2);} }
}
package mmo.petfight.bitmap {
    import flash.display.BitmapData;
    public class pet6044_1_1_1 extends BitmapData { public function pet6044_1_1_1(){super(2,2);} }
}
package mmo.petfight.bitmap {
    import flash.display.BitmapData;
    public class pet31_1_1_0 extends BitmapData { public function pet31_1_1_0() { super(2,2); } }
}
package mmo.petfight.bitmap {
    import flash.display.BitmapData;
    public class pet31_1_1_1 extends BitmapData { public function pet31_1_1_1() { super(2,2); } }
}
package mmo.loader.scheduler { public class ResPriority { public static const PRELOAD:int=2; } }
package mmo.loader.scheduler {
    public class ResScheduler {
        public static var urls:Array = [];
        public static function getResource(url:String,done:Function,cls:String,failed:Function,progress:Function,priority:int,n:Boolean,domain:Object):void {
            if(priority != ResPriority.PRELOAD) throw new Error("priority");
            urls.push(url); done();
        }
        public static function cancel(url:String,fn:Function):void {}
    }
}
package mmo.pet { import flash.display.Sprite; public class peticon29_F extends Sprite {} }
package mmo.petfightnew.common.manager {
    public class ServicePool {
        public static var instance:ServicePool = new ServicePool();
        public function getService(type:Object):Object { return {customData:{pmDataMap:{getPM:function(id:int):Boolean { return true; }}}}; }
    }
}
package mmo.interfaces.pet.pm.services { public interface IPMService {} }
