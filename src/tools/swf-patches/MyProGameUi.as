package aola.tools {
    import flash.external.ExternalInterface;
    import flash.system.ApplicationDomain;
    import flash.display.DisplayObject;
    import flash.utils.getQualifiedClassName;

    public final class MyProGameUi {
        private static var controlDomain:ApplicationDomain;
        public static function bindControl(domain:ApplicationDomain):void { controlDomain=domain; }
        public static function install():void {
            try {
                if (!ExternalInterface.available) return;
                ExternalInterface.addCallback("onAolaOpenPanel", openPanel);
                ExternalInterface.addCallback("onAolaThemeApply", applyTheme);
                ExternalInterface.addCallback("onAolaThemeRead", readTheme);
            } catch (ignored:Error) {}
        }

        private static function definition(name:String):Object {
            var domain:ApplicationDomain = ApplicationDomain.currentDomain;
            if (controlDomain && controlDomain.hasDefinition(name)) return controlDomain.getDefinition(name);
            return domain.hasDefinition(name) ? domain.getDefinition(name) : null;
        }

        private static function openPanel(id:String):String {
            var resource:String;
            var cls:String;
            switch (id) {
                case "theme-settings":
                    resource = "activityres/activity240927/controlbarskinsetting/controlbarskinsetting_code";
                    cls = "mmo.activity240927.controlbarskinsetting.panel.CBSS_MainPl";
                    break;
                case "summoner":
                    resource = "activityres/activity250516/newzhaohuanshi/newzhaohuanshi_code";
                    cls = "mmo.activity250516.newzhaohuanshi.panel.NZHS_MainPl";
                    break;
                default:
                    return "blocked";
            }
            try {
                var helper:Object = definition("mmo.play.activity.helper.Panel_Helper");
                var services:Object = definition("mmo.interfaces.ServiceContainer");
                var fightClass:Object = definition("mmo.interfaces.pet.fight.IPetFight");
                if (!helper || !services || !fightClass) return "unavailable";
                var fight:Object = services.getService(fightClass);
                if (!fight) return "unavailable";
                if (fight.isFighting()) return "blocked";
                helper.loadResAndShow(resource,cls);
                return "requested";
            } catch (error:Error) { return "failed"; }
        }

        private static function applyTheme(id:int, request:int):String {
            var status:String = "NotReady";
            var reason:String = "";
            if (id < 0 || id > 10 || request < 1) status = "Failed";
            else {
                try {
                    var handler:Object = definition("mmo.controlv2.controlbarskin.ControlBarSkinHandler");
                    if (handler && handler["applyTheme"] is Function) return handler.applyTheme(id, request);
                } catch (error:Error) { status = "Failed"; reason = error.message; }
            }
            return JSON.stringify({themeId:id,requestId:request,status:status,message:reason});
        }

        private static function readTheme():int {
            try {
                var handler:Object = definition("mmo.controlv2.controlbarskin.ControlBarSkinHandler");
                var services:Object = definition("mmo.interfaces.ServiceContainer");
                var controlClass:Object = definition("mmo.interfaces.control.IControlService");
                if (!handler || !(handler["applyTheme"] is Function) || !services || !controlClass) return -1;
                var control:Object = services.getService(controlClass);
                var node:DisplayObject = control ? control.commonMapButton as DisplayObject : null;
                if (!node || !node.stage) return -1;
                while (node) {
                    var name:String = getQualifiedClassName(node);
                    for (var id:int=1; id<=10; id++) {
                        if (name == "aola.theme.t"+id+".mmo.controlv2.controlbarskin.skin::Skin"+id) return id;
                    }
                    node = node.parent;
                }
                return 0;
            } catch (ignored:Error) {}
            return -1;
        }
    }
}

aola.tools.MyProGameUi.install();
