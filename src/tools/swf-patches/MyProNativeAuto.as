package aola.tools {
    import flash.display.MovieClip;
    import flash.display.DisplayObjectContainer;
    import flash.events.MouseEvent;
    import flash.external.ExternalInterface;
    import flash.utils.Dictionary;

    public class MyProNativeAuto {
        private static const buttons:Dictionary = new Dictionary(true);
        private static var controller:Class;
        private static var data:Class;

        public static function bind(button:MovieClip, fightController:Class, fightData:Class):void {
            if (!button || !ExternalInterface.available) return;
            buttons[button] = true;
            controller = fightController;
            data = fightData;
            ExternalInterface.addCallback("getNativeAutoBattle", getState);
            ExternalInterface.addCallback("setNativeAutoBattle", setState);
        }

        public static function unbind(button:MovieClip):void {
            if (!button || !buttons[button]) return;
            delete buttons[button];
            for (var other:Object in buttons) return;
            controller = null;
            data = null;
            if (ExternalInterface.available) {
                ExternalInterface.addCallback("setNativeAutoBattle", null);
                ExternalInterface.addCallback("getNativeAutoBattle", null);
            }
        }

        private static function getState():Object { return state(null); }
        private static function setState(enabled:*):Object {
            return enabled is Boolean ? state(enabled) : {status:"blocked",enabled:null,changed:false};
        }

        private static function state(request:*):Object {
            try {
                var fight:Object = controller ? controller.instance : null;
                if (fight && fight.fightPanel && fight.whoWon == -1 && !fight.isReplay) {
                    var forced:Boolean = data.instance.isPVPForceAuto;
                    var enabled:Boolean = (forced && fight.canAuto) || fight.useAiType == 2 ||
                        (fight.useAiType != 3 && Boolean(controller.isAutoUseSkill));
                    if (request !== null && enabled == request)
                        return {status:"applied",enabled:enabled,changed:false};
                    var button:MovieClip;
                    for (var item:Object in buttons) {
                        var candidate:MovieClip = item as MovieClip;
                        if (!candidate.stage || !candidate.visible || !candidate.mouseEnabled) continue;
                        var parent:DisplayObjectContainer = candidate.parent;
                        while (parent && parent.visible && parent.mouseChildren) parent = parent.parent;
                        if (!parent) { button = candidate; break; }
                    }
                    if (!button || !fight.canAuto || fight.isAuto || forced || fight.useAiType == 2 || fight.useAiType == 3)
                        return {status:"blocked",enabled:enabled,changed:false};
                    if (request === null) return {status:"ready",enabled:enabled,changed:false};
                    // The existing listener owns autoUseAI, disabling, strategy reset and the button refresh.
                    button.dispatchEvent(new MouseEvent(MouseEvent.CLICK, false));
                    var actual:Boolean = Boolean(controller.isAutoUseSkill);
                    return {status:actual == request ? "applied" : "blocked",enabled:actual,changed:actual != enabled};
                }
            } catch (ignored:Error) {}
            return {status:"unavailable",enabled:null,changed:false};
        }
    }
}
