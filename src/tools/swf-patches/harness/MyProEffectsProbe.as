package {
    import flash.display.Sprite;
    import probe.GameEffects;
    import probe.external.ExternalInterface;
    public class MyProEffectsProbe extends Sprite {
        public function MyProEffectsProbe() {
            var p:GameEffects = new GameEffects();
            function sample(label:String):void {
                p.events = [];
                p.hit(); p.heal();
                for each (var v:int in [-2, 0, 2]) { p._hpInc = v; p.multi(); }
                p.change(11); p.skill(); p.next();
                trace(label + " " + p.events.join(",") + " delay=" + p.delay());
            }
            sample("default");
            ExternalInterface.flags.aolaHideBattleDamage = true; sample("damage");
            ExternalInterface.flags.aolaHideBattleDamage = false;
            ExternalInterface.flags.aolaHideBattleHealing = true; sample("healing");
            ExternalInterface.flags.aolaHideBattleHealing = false;
            ExternalInterface.flags.aolaSkipBattleAnimations = true; sample("skip");
            ExternalInterface.flags.aolaHideBattleDamage = true;
            ExternalInterface.flags.aolaHideBattleHealing = true; sample("all");
            ExternalInterface.available = false; sample("host-absent");
            ExternalInterface.available = true; ExternalInterface.fail = true; sample("host-error");
        }
    }
}
package probe.external {
    public class ExternalInterface {
        public static var available:Boolean = true;
        public static var fail:Boolean = false;
        public static var flags:Object = {};
        public static function call(name:String):* {
            if (fail) throw new Error("fixture host error");
            return flags[name];
        }
    }
}
package probe {
    public class GameEffects {
        public var events:Array = [];
        public var _hpInc:int;
        public var skillBS:int = 1, _bus:int = 2, _skillParams:int = 3;
        public var fightPanel:GameEffects = this;
        public function hit():void { events.push("d"); }
        public function heal():void { events.push("h"); }
        public function multi():void { events.push("m" + _hpInc); }
        public function change(slot:int):void { events.push("switchMovie"); }
        public function skill():void { events.push("skillMovie"); }
        public function next():void { events.push("turnMovie"); }
        public function delay():int { return 16; }
        public function onSwitchEnd(slot:int):void { events.push("switchEnd" + slot); }
        public function refreshInfos():void { events.push("refresh"); }
        public function setBS(n:int):void { events.push("bs" + n); }
        public function setBackGroundUp(n:int):void { events.push("bg" + n); }
        public function setPetUp(n:int):void { events.push("up" + n); }
        public function handleSkillEnd():void { events.push("skillEnd"); }
        public function handle():void { events.push("turnEnd"); }
    }
}
package mmo.petfightnew.common.manager {
    public class FightFrameRateManager {
        public static var instance:FightFrameRateManager = new FightFrameRateManager();
        public function correctFrameRate():void { trace("correct-rate"); }
    }
}
