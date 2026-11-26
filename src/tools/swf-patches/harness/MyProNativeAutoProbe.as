package {
    import flash.display.MovieClip;
    import flash.display.Sprite;
    import flash.events.Event;
    import flash.events.MouseEvent;
    import flash.utils.getDefinitionByName;
    import probe.AutoController;
    import probe.AutoData;
    import probe.external.ExternalInterface;

    public class MyProNativeAutoProbe extends Sprite {
        private var clicks:int;
        public function MyProNativeAutoProbe() { addEventListener(Event.ENTER_FRAME, probe); }
        private function probe(e:Event):void {
            removeEventListener(Event.ENTER_FRAME, probe);
            var helper:Class = getDefinitionByName("aola.tools.MyProNativeAuto") as Class;
            var button:MovieClip = new MovieClip();
            var first:MovieClip = new MovieClip();
            var panel:Sprite = new Sprite();
            addChild(panel);
            panel.addChild(button);
            button.addEventListener(MouseEvent.CLICK, function(event:MouseEvent):void {
                clicks++;
                AutoController.isAutoUseSkill = !AutoController.isAutoUseSkill;
                if (event.bubbles) trace("unexpected bubbling");
            });
            ExternalInterface.available = false;
            helper.bind(button, AutoController, AutoData);
            trace("no host", !ExternalInterface.callbacks.setNativeAutoBattle);
            ExternalInterface.available = true;
            helper.bind(first, AutoController, AutoData);
            helper.bind(button, AutoController, AutoData);
            helper.unbind(first);
            var getState:Function = ExternalInterface.callbacks.getNativeAutoBattle;
            var setState:Function = ExternalInterface.callbacks.setNativeAutoBattle;
            check("initial", getState());
            check("enable", setState(true));
            check("same", setState(true));
            check("disable", setState(false));
            check("wrong type", setState(1));
            var fight:Object = AutoController.instance;
            fight.canAuto = false;
            check("cannot auto", setState(true));
            fight.canAuto = true;
            fight.isAuto = true;
            check("locked", setState(true));
            fight.isAuto = false;
            AutoData.instance.isPVPForceAuto = true;
            check("forced", setState(false));
            AutoData.instance.isPVPForceAuto = false;
            fight.useAiType = 2;
            check("mode2", setState(false));
            fight.useAiType = 3;
            check("mode3", setState(true));
            fight.useAiType = 0;
            fight.whoWon = 1;
            check("won", setState(true));
            fight.whoWon = -1;
            fight.isReplay = true;
            check("replay", setState(true));
            fight.isReplay = false;
            fight.fightPanel = null;
            check("no fight", getState());
            fight.fightPanel = {};
            button.mouseEnabled = false;
            check("disabled button", setState(true));
            button.mouseEnabled = true;
            panel.visible = false;
            check("hidden panel", setState(true));
            panel.visible = true;
            panel.mouseChildren = false;
            check("disabled panel", setState(true));
            panel.mouseChildren = true;
            removeChild(panel);
            check("detached", setState(true));
            helper.unbind(button);
            trace("disposed", !ExternalInterface.callbacks.setNativeAutoBattle,
                !ExternalInterface.callbacks.getNativeAutoBattle, clicks);
        }
        private function check(label:String, result:Object):void {
            trace(label, result.status, result.enabled, result.changed, clicks);
        }
    }
}
package probe {
    public class AutoController {
        public static var instance:Object = {fightPanel:{}, whoWon:-1, isReplay:false, canAuto:true, isAuto:false, useAiType:0};
        public static var isAutoUseSkill:Boolean = false;
    }
}
package probe {
    public class AutoData {
        public static var instance:Object = {isPVPForceAuto:false};
    }
}
package probe.external {
    public class ExternalInterface {
        public static var available:Boolean = true;
        public static var callbacks:Object = {};
        public static function addCallback(name:String, handler:Function):void {
            if (handler == null) delete callbacks[name];
            else callbacks[name] = handler;
        }
    }
}
