package {
    import flash.display.Sprite;
    import flash.utils.getDefinitionByName;
    import mmo.petfightnew.common.manager.ServicePool;
    import probe.BattleHost;

    public class BattleProbe extends Sprite {
        public function BattleProbe() {
            var helper:Class = getDefinitionByName("mmo.petfightnew.common.util.GQWanyuZhichaoEffectHelper") as Class;
            var completions:int = 0;
            var done:Function = function():void { completions++; };
            BattleHost.block = true;
            helper.playAttackMovie(null, 0, 0, done);
            helper.playAttackMovie(null, 0, 0, null);
            trace("blocked", completions, ServicePool.lookups, BattleHost.calls);
            BattleHost.block = false;
            helper.playAttackMovie(null, 0, 0, done);
            helper.playAttackMovie(null, 0, 0, null);
            trace("original", completions, ServicePool.lookups, BattleHost.calls);
        }
    }
}
package probe {
    public class BattleHost {
        public static var block:Boolean;
        public static var calls:int;
        public static function call(name:String, channel:String):Boolean {
            if (name != "aolaBlockSkill" || channel != "wanyu-zhichao") throw new Error("wrong host callback");
            calls++;
            return block;
        }
    }
}
package mmo.petfightnew.common.manager {
    public class ServicePool {
        public static var lookups:int;
        public static var instance:ServicePool = new ServicePool();
        public function getService(type:Class):Object {
            lookups++;
            return null;
        }
    }
}
package mmo.petfightnew.common {
    public class FightData {
        public static var instance:FightData = new FightData();
        public var teammates:Array = [];
        public var opponents:Array = [];
        public function isMySide(location:int):Boolean { return true; }
    }
}
// Only the slot type identities are needed to verify the original activation.
package mmo.interfaces.pet.data { public class PlayerBattleInfo {} }
package mmo.interfaces.pet.data { public class PetBriefInfo {} }
package mmo.interfaces.pet.pm.services { public interface IPMService {} }
