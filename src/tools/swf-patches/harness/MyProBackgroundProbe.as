package {
    import flash.display.Bitmap;
    import flash.display.BitmapData;
    import flash.display.Sprite;
    import flash.events.Event;
    import flash.utils.getDefinitionByName;
    import probe.external.ExternalInterface;

    public class MyProBackgroundProbe extends Sprite {
        private var api:Class;
        private var layer:Sprite = new Sprite();
        private var stands:Sprite = new Sprite();
        private var phase:int;

        public function MyProBackgroundProbe() {
            api = getDefinitionByName("aola.tools.MyProBattleBackground") as Class;
            layer.addChild(new Sprite());
            stands.visible = false;
            api.bind(layer, stands);
            trace("bind", ExternalInterface.reads, ExternalInterface.callback != null);
            var ended:Sprite = layer;
            api.detach(ended);
            layer = new Sprite();
            layer.addChild(new Sprite());
            api.bind(layer, stands);
            trace("pending scene change", ended.numChildren, ExternalInterface.reads);
            addEventListener(Event.ENTER_FRAME, next);
        }
        private function next(e:Event):void {
            if (phase == 0 && layer.numChildren == 2) {
                var pixels:BitmapData = Bitmap(layer.getChildAt(1)).bitmapData;
                trace("decoded", pixels.width, pixels.height, (pixels.getPixel(30,30) >> 16) > 200, stands.visible);
                var skill:Sprite = new Sprite();
                layer.addChild(skill);
                trace("skill layer", layer.getChildAt(2) is Bitmap, layer.contains(skill));
                api.beforeBS(layer);
                trace("official before update", stands.visible);
                stands.visible = false;
                api.afterBS(layer);
                trace("custom after update", stands.visible);
                api.detach(layer);
                trace("detached", layer.numChildren, stands.visible, ExternalInterface.callback == null);
                layer = new Sprite();
                layer.addChild(new Sprite());
                api.bind(layer, stands);
                trace("second battle", Bitmap(layer.getChildAt(1)).bitmapData === pixels, ExternalInterface.reads);
                ExternalInterface.jpeg = "";
                ++ExternalInterface.revision;
                ExternalInterface.callback();
                trace("restore", layer.numChildren, stands.visible, ExternalInterface.reads);
                ExternalInterface.jpeg = "AAAA";
                ++ExternalInterface.revision;
                ExternalInterface.callback();
                phase = 1;
            } else if (phase == 1) {
                trace("invalid", layer.numChildren, stands.visible, ExternalInterface.reads);
                ExternalInterface.callback();
                trace("unchanged revision", ExternalInterface.reads);
                ExternalInterface.jpeg = ExternalInterface.fixture;
                ++ExternalInterface.revision;
                ExternalInterface.callback();
                phase = 2;
            } else if (phase == 2 && layer.numChildren == 2) {
                trace("new image", stands.visible, ExternalInterface.reads);
                api.detach(layer);
                removeEventListener(Event.ENTER_FRAME, next);
            }
        }
    }
}
package probe.external {
    public class ExternalInterface {
        public static var available:Boolean = true;
        public static var revision:int = 1;
        public static const fixture:String = "@JPEG@";
        public static var jpeg:String = fixture;
        public static var reads:int;
        public static var callback:Function;
        public static function addCallback(key:String, fn:Function):void {
            if (key == "aolaRefreshBattleBackground") callback = fn;
        }
        public static function call(key:String):* {
            if (key == "aolaBattleBackgroundRevision") return revision;
            if (key == "aolaBattleBackgroundJpeg") { ++reads; return jpeg; }
            throw new Error(key);
        }
    }
}
