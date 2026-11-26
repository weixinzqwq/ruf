package {
    import flash.display.*;
    import flash.events.*;
    import flash.net.URLRequest;
    import flash.system.*;
    import probe.external.ExternalInterface;

    public class MyProGameUiProbe extends MovieClip {
        private var game:Loader;
        private var leaf:Loader;
        private var id:int;
        private var callbacks:Object;

        public function MyProGameUiProbe() { addEventListener(Event.ENTER_FRAME, start); }
        private function start(e:Event):void {
            removeEventListener(Event.ENTER_FRAME,start);
            callbacks=ExternalInterface.callbacks;
            trace("empty",callbacks.onAolaThemeRead(),callbacks.onAolaOpenPanel("theme-settings"));
            game=new Loader();
            addChild(game);
            game.contentLoaderInfo.addEventListener(Event.COMPLETE,ready);
            game.load(new URLRequest("control.swf"),new LoaderContext(false,new ApplicationDomain(ApplicationDomain.currentDomain)));
        }
        private function ready(e:Event):void {
            trace("child only",ApplicationDomain.currentDomain.hasDefinition("mmo.play.activity.helper.Panel_Helper"),
                  callbacks.onAolaThemeRead());
            trace("settings",callbacks.onAolaOpenPanel("theme-settings"),Object(game.content).lastPanel);
            trace("summoner",callbacks.onAolaOpenPanel("summoner"),Object(game.content).lastPanel);
            trace("unknown",callbacks.onAolaOpenPanel("unknown"));
            Object(game.content).fighting=true;
            trace("battle",JSON.parse(callbacks.onAolaThemeApply(10,1)).status,callbacks.onAolaOpenPanel("summoner"));
            Object(game.content).fighting=false;
            trace("after",JSON.parse(callbacks.onAolaThemeApply(10,1)).status,callbacks.onAolaThemeRead());
            trace("native",JSON.parse(callbacks.onAolaThemeApply(0,2)).status,callbacks.onAolaThemeRead());
            next();
        }
        private function next():void {
            if (++id>10) return;
            leaf=new Loader();
            leaf.contentLoaderInfo.addEventListener(Event.COMPLETE,check);
            leaf.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR,function(e:Event):void {trace("leaf error",id);});
            leaf.load(new URLRequest("skin"+id+".swf"),new LoaderContext(false,new ApplicationDomain(ApplicationDomain.currentDomain)));
        }
        private function check(e:Event):void {
            var name:String="aola.theme.t"+id+".mmo.controlv2.controlbarskin.skin.Skin"+id;
            var cls:Class=leaf.contentLoaderInfo.applicationDomain.getDefinition(name) as Class;
            var view:MovieClip=new cls() as MovieClip;
            trace("leaf",id,view!=null,view.getChildByName("btnMap")!=null);
            leaf.unload();
            next();
        }
    }
}
package probe.external {
    public final class ExternalInterface {
        public static var callbacks:Object={};
        public static var available:Boolean=true;
        public static function addCallback(name:String,fn:Function):void {callbacks[name]=fn;}
    }
}
