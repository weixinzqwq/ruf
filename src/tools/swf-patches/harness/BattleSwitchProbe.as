package {
    import flash.display.Sprite;
    import flash.events.Event;
    import flash.utils.getDefinitionByName;
    import mmo.petfightnew.common.*;

    public class BattleSwitchProbe extends Sprite {
        private var panel:FightPanel = new FightPanel();
        private var switcher:Object;
        private var step:int;

        public function BattleSwitchProbe() {
            var cls:Class = getDefinitionByName("mmo.petfightnew.common.fightcontroller.SwitchPm") as Class;
            switcher = new cls();
            addChild(panel.layerPet);
            addEventListener(Event.ENTER_FRAME, advance);
        }
        private function advance(e:Event):void {
            ++step;
            if (step == 30 || step == 60) {
                switcher.init(panel);
                trace("switched", panel.layerPet.numChildren, panel.petMCList[0].loaded);
            }
            panel.layerPet.x = step % 20;
        }
    }
}
package mmo.petfightnew.common {
    import flash.display.Sprite;
    public class FightPanel {
        public var layerPet:Sprite = new Sprite();
        public var layerEffect:Sprite = new Sprite();
        public var petMCList:Array = [];
        public var petLoadeNum:int = 2;
        public function FightPanel() {
            petMCList[0] = new MCPetContainer(null, false, 0, layerEffect);
            layerPet.addChild(petMCList[0]);
        }
    }
}
package mmo.petfightnew.common {
    import flash.display.Sprite;
    import flash.filters.GlowFilter;
    public class MCPetContainer extends Sprite {
        public var loaded:Boolean;
        public function MCPetContainer(pet:Object, enemy:Boolean, pos:int, layer:Sprite) {
            graphics.beginFill(0xff0000);
            graphics.drawCircle(30, 30, 15);
            graphics.endFill();
            filters = [new GlowFilter()];
        }
        public function dispose():void { filters = []; }
        public function startLoad():void { loaded = true; }
    }
}
package mmo.petfightnew.common {
    public class FightData {
        public static const instance:FightData = new FightData();
        public function isMySide(pos:int):Boolean { return false; }
        public var myInfo:Object;
    }
}
package mmo.common {
    import flash.display.DisplayObject;
    public class DisplayUtil {
        public static function stopAndRemove(sprite:DisplayObject):void { sprite.parent.removeChild(sprite); }
    }
}
package mmo.petfightnew.common.util {
    import flash.geom.Point;
    public class PositionUtil {
        public static function getDefaultPosition(pos:int):Point { return new Point(12, 12); }
    }
}
package mmo.interfaces.pet.fight {
    import flash.events.Event;
    public class PetFightEvent extends Event {
        public static const GOT_PET:String = "gotPet";
        public function PetFightEvent(type:String) { super(type); }
    }
}
package mmo.interfaces.pet.data {
    public class PetBriefInfo {}
}
