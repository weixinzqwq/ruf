package {
    import flash.display.Sprite;
    import mmo.interfaces.ServiceContainer;
    import mmo.play.activity.helper.Panel_Helper;
    import mmo.controlv2.controlbarskin.ControlBarSkinHandler;
    public class MyProGameUiControlProbe extends Sprite {
        public function MyProGameUiControlProbe() {
            ControlBarSkinHandler.root=this;
            ControlBarSkinHandler.select(0);
        }
        public function set fighting(value:Boolean):void {ServiceContainer.fight=value;}
        public function get lastPanel():String {return Panel_Helper.last;}
    }
}
package mmo.interfaces.pet.fight { public interface IPetFight {} }
package mmo.interfaces.control { public interface IControlService {} }
package mmo.interfaces {
    import mmo.interfaces.pet.fight.IPetFight;
    public class ServiceContainer {
        public static var fight:Boolean=false;
        public static var button:Object;
        public static function getService(cls:Object):Object {
            if (cls===IPetFight) return {isFighting:function():Boolean {return fight;}};
            return {commonMapButton:button};
        }
    }
}
package mmo.play.activity.helper {
    public class Panel_Helper {
        public static var last:String;
        public static function loadResAndShow(resource:String,cls:String):void { last=resource+" "+cls; }
    }
}
package mmo.controlv2.controlbarskin {
    import aola.theme.t10.mmo.controlv2.controlbarskin.skin.Skin10;
    import flash.display.Sprite;
    import mmo.interfaces.ServiceContainer;
    public class ControlBarSkinHandler {
        public static var root:Sprite;
        public static function select(id:int):void {
            if(root.numChildren) root.removeChildAt(0);
            var view:Sprite=id==10 ? new Skin10() : new Sprite();
            root.addChild(view);
            ServiceContainer.button=view.addChild(new Sprite());
        }
        public static function applyTheme(id:int,request:int):String {
            if(!ServiceContainer.fight) select(id);
            return JSON.stringify({status:ServiceContainer.fight ? "WaitingBattleEnd" : "Applied",themeId:id,requestId:request});
        }
    }
}
package aola.theme.t10.mmo.controlv2.controlbarskin.skin {
    import flash.display.Sprite;
    public class Skin10 extends Sprite {}
}
