package {
    import flash.display.MovieClip;
    import flash.display.Sprite;
    import flash.system.ApplicationDomain;
    import flash.utils.getDefinitionByName;
    import flash.utils.Timer;
    import mmo.common.simplemoviehelper.ShowMovieOrCartoonHelper;
    import mmo.loader.layer.LayerManager;
    import mmo.petfightnew.common.FightData;
    import mmo.petfightnew.common.FightPanel;
    import probe.BackdropHost;

    public class BattleBackdropProbe extends Sprite {
        public function BattleBackdropProbe() {
            var pkg:String = "mmo.petfightnew.common.startmovie.";
            var normal:Boolean = ApplicationDomain.currentDomain.hasDefinition(pkg + "StartMovie");
            var name:String = normal ? "StartMovie" : "StartMovie3v3";
            var entry:Class = getDefinitionByName(pkg + name) as Class;
            try {
                for each (var blocked:Boolean in [false, true]) {
                    for (var flags:int = 0; flags < 4; flags++) exercise(entry, normal, blocked, flags);
                }
                trace(name, "8 cases passed");
            } catch (e:Error) {
                trace("FAILED", e, e.getStackTrace());
            }
        }

        private function exercise(entry:Class, normal:Boolean, blocked:Boolean, flags:int):void {
            var view:Object = normal ? new entry(new MovieClip(), [], []) : new entry();
            var movie:MovieClip = normal ? MovieClip(view) : new MovieClip();
            if (!normal) view.startMovie = movie;
            addChild(movie);
            var timer:Timer = new Timer(60000);
            view.timer = timer;
            timer.start();
            FightData.instance.haveGQPet = (flags & 1) != 0;
            FightData.instance.haveXinghuiPet = (flags & 2) != 0;
            BackdropHost.block = blocked;
            BackdropHost.steps = [];
            LayerManager.layer = new Sprite();
            ShowMovieOrCartoonHelper.done = null;
            var channel:String = normal && (flags & 1) ? "gq" : (flags & 2) ? "xinghui" : "";
            var expected:Array = ["remove", "dispose"];
            view.enterFight();
            if (timer.running || movie.parent) throw new Error("dispose did not stop the timer and remove the movie");
            if (channel) expected.push("battle-backdrop-" + channel);
            if (channel && !blocked) {
                expected.push(channel == "gq" ? "mmo.petfight.GQcommonBackMv" : "mmo.petfight.XinghuicommonBackMv");
                if (BackdropHost.steps.join() != expected.join() || LayerManager.layer.numChildren != 0)
                    throw new Error("completion ran before the background finished");
                ShowMovieOrCartoonHelper.done();
            } else if (ShowMovieOrCartoonHelper.done != null) {
                throw new Error("unexpected background request");
            }
            expected.push("panel", "fight-ui-start");
            if (BackdropHost.steps.join() != expected.join() || LayerManager.layer.numChildren != 1
                    || !(LayerManager.layer.getChildAt(0) is FightPanel))
                throw new Error("expected " + expected + "; observed " + BackdropHost.steps);
        }
    }
}
package probe {
    public class BackdropHost {
        public static var block:Boolean;
        public static var steps:Array;
        public static function call(name:String, channel:String):Boolean {
            if (name != "aolaBlockSkill") throw new Error("unknown host call");
            steps.push(channel);
            return block;
        }
    }
}
package mmo.petfightnew.common {
    public class FightData {
        public static var instance:FightData = new FightData();
        public var haveGQPet:Boolean;
        public var haveXinghuiPet:Boolean;
    }
}
package mmo.petfightnew.common {
    import flash.display.Sprite;
    import probe.BackdropHost;
    public class FightPanel extends Sprite {
        public function FightPanel() { BackdropHost.steps.push("panel"); }
    }
}
package mmo.common.simplemoviehelper {
    import probe.BackdropHost;
    public class ShowMovieOrCartoonHelper {
        public static var done:Function;
        public static function showMovieOrCartoon(show:Boolean, path:String, asset:String, complete:Function):void {
            if (!show || path != "petfightbackground/common" || done != null) throw new Error("unexpected request");
            BackdropHost.steps.push(asset);
            done = complete;
        }
    }
}
package mmo.loader.layer {
    import flash.display.Sprite;
    public class LayerManager {
        public static var layer:Sprite;
        public static function getFightingTop():Sprite { return layer; }
    }
}
package mmo.interfaces.pet.fight {
    import flash.events.Event;
    public class PetFightEvent extends Event {
        public static const FIGHT_UI_START:String = "fight-ui-start";
        public function PetFightEvent(type:String, data:Object) { super(type); }
    }
}
package mmo.common.eventdispatcher {
    import flash.events.Event;
    import flash.events.EventDispatcher;
    import probe.BackdropHost;
    public class CommonEventDispatcher extends EventDispatcher {
        public static var instance:CommonEventDispatcher = new CommonEventDispatcher();
        override public function dispatchEvent(event:Event):Boolean {
            BackdropHost.steps.push(event.type);
            return true;
        }
    }
}
package mmo.common {
    import flash.display.MovieClip;
    import probe.BackdropHost;
    public class DisplayUtil {
        public static function stopAndRemove(movie:MovieClip):void {
            movie.stop();
            movie.parent.removeChild(movie);
            BackdropHost.steps.push("remove");
        }
    }
}
package mmo.petfightnew.common.util {
    import probe.BackdropHost;
    public class PreLoadAssetsUtil {
        public static var instance:PreLoadAssetsUtil = new PreLoadAssetsUtil();
        public function dispose():void { BackdropHost.steps.push("dispose"); }
    }
}
package mmo.petfightnew.common.startmovie {
    public class PlayerInfoHandler { public static const LOAD_DONE:String = "done"; }
}
package mmo.framework.comm {
    import flash.events.EventDispatcher;
    public class SocketClient extends EventDispatcher { public static var instance:SocketClient = new SocketClient(); }
}
package mmo.framework.comm {
    public class PetClient { public static const RSPN_ESCAPE:String = "escape"; }
}
package mmo.framework.comm {
    import flash.events.Event;
    public class SocketClientEvent extends Event { public function SocketClientEvent(type:String) { super(type); } }
}
