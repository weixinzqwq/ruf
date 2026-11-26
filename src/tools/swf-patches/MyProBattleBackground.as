package aola.tools {
    import flash.display.Bitmap;
    import flash.display.BitmapData;
    import flash.display.DisplayObjectContainer;
    import flash.display.Loader;
    import flash.events.Event;
    import flash.events.IOErrorEvent;
    import flash.external.ExternalInterface;
    import flash.utils.ByteArray;

    public final class MyProBattleBackground {
        private static var layer:DisplayObjectContainer;
        private static var platform:DisplayObjectContainer;
        private static var officialVisible:Boolean;
        private static var overlay:Bitmap;
        private static var loading:Loader;
        private static var revision:Number = -1;
        private static var cached:BitmapData;

        public static function bind(background:DisplayObjectContainer, stands:DisplayObjectContainer):void {
            if (!background || !ExternalInterface.available) return;
            if (background !== layer) {
                detach(layer);
                layer = background;
                platform = stands;
                try { ExternalInterface.addCallback("aolaRefreshBattleBackground", refresh); }
                catch (e:Error) {}
            }
            refresh();
        }

        public static function detach(background:DisplayObjectContainer):void {
            if (!layer || layer !== background) return;
            hide();
            // The private loader holds only the JPEG. A pending decode can fill the
            // cache after a battle ends, without retaining that battle's display tree.
            layer = null;
            platform = null;
            try { ExternalInterface.addCallback("aolaRefreshBattleBackground", null); }
            catch (e:Error) {}
        }

        // Allow the game to maintain its own platform and skill background even under the custom image.
        public static function beforeBS(background:DisplayObjectContainer):void {
            if (background === layer && overlay && platform) platform.visible = officialVisible;
        }
        public static function afterBS(background:DisplayObjectContainer):void {
            if (background === layer && overlay && platform) {
                officialVisible = platform.visible;
                platform.visible = true;
            }
        }

        private static function keepOnTop(e:Event):void {
            if (overlay && overlay.parent === layer && overlay !== e.target
                && layer.getChildIndex(overlay) != layer.numChildren - 1)
                layer.setChildIndex(overlay, layer.numChildren - 1);
        }

        private static function hide():void {
            if (!overlay) return;
            layer.removeEventListener(Event.ADDED, keepOnTop);
            if (overlay.parent) overlay.parent.removeChild(overlay);
            overlay.bitmapData = null;
            overlay = null;
            if (platform) platform.visible = officialVisible;
        }

        private static function show():void {
            if (!layer || overlay || !cached) return;
            overlay = new Bitmap(cached);
            layer.addChild(overlay);
            layer.addEventListener(Event.ADDED, keepOnTop);
            if (platform) {
                officialVisible = platform.visible;
                platform.visible = true;
            }
        }

        private static function cancel():void {
            if (!loading) return;
            loading.contentLoaderInfo.removeEventListener(Event.COMPLETE, loaded);
            loading.contentLoaderInfo.removeEventListener(IOErrorEvent.IO_ERROR, failed);
            loading.unload();
            loading = null;
        }
        private static function failed(e:Event):void {
            if (loading && e.currentTarget === loading.contentLoaderInfo) cancel();
        }
        private static function loaded(e:Event):void {
            if (!loading || e.currentTarget !== loading.contentLoaderInfo) return;
            var image:Bitmap = loading.content as Bitmap;
            if (image && image.bitmapData && image.bitmapData.width == 960 && image.bitmapData.height == 560)
                cached = image.bitmapData;
            cancel();
            show();
        }

        public static function refresh():void {
            if (!layer) return;
            try {
                var next:Number = Number(ExternalInterface.call("aolaBattleBackgroundRevision"));
                if (revision == next) { show(); return; }
                revision = next;
                hide();
                cancel();
                if (cached) { cached.dispose(); cached = null; }
                var source:String = ExternalInterface.call("aolaBattleBackgroundJpeg") as String;
                if (!source) return;
                var bytes:ByteArray = new ByteArray();
                bytes.aoqiDecodeBase64(source);
                bytes.position = 0;
                loading = new Loader();
                loading.contentLoaderInfo.addEventListener(Event.COMPLETE, loaded);
                loading.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR, failed);
                loading.loadBytes(bytes);
            } catch (e:Error) { cancel(); }
        }
    }
}
