package {
    import flash.display.MovieClip;
    import flash.events.Event;
    import flash.utils.getDefinitionByName;
    import mmo.loader.scheduler.ResScheduler;
    import probe.ModelHost;

    public class MyProBackpack6044Probe extends MovieClip {
        private var helper:Object;
        public function MyProBackpack6044Probe() {
            helper = getDefinitionByName("aola.tools.MyProBackpackMorph");
            trace("cold", helper.resolve(5336), ModelHost.reason);
            trace("unrelated", helper.resolve(5337));
            addEventListener(Event.ENTER_FRAME,step);
        }
        private function step(event:Event):void {
            if (!ModelHost.loaded) return;
            removeEventListener(Event.ENTER_FRAME,step);
            trace("ready", helper.resolve(5336), "parts", ResScheduler.requests);
        }
    }
}
package probe {
    public class ModelHost {
        public static var available:Boolean = true;
        public static var loaded:Boolean;
        public static var reason:String;
        public static function call(name:String,...args):* {
            if(name=="aolaMyProBinding") return 17;
            if(name=="aolaMyProModelCandidate") return args[1]==5336 ? 6044 : args[1];
            if(name=="aolaMyProModelUse") {reason=args[5];return args[4] ? args[2] : args[1];}
            if(name=="aolaMyProModelProgress") {
                trace("progress",args[5],args[6],args[7]);
                loaded=args[5]=="loaded";
            }
        }
    }
}
package mmo.interfaces {
    public class ServiceContainer {
        public static var instance:Object = {getService:function(type:*):Object{return {
            getOperationPM:function(id:int):*{return id==6044 ? {pmData:{}} : null;},
            customData:{pmDataMap:{getPM:function(id:int):*{return id==6044 ? {} : null;}}}
        };}};
    }
}
package mmo.interfaces.pet.pm.services { public interface IPMService {} }
package mx.core {
    import flash.utils.ByteArray;
    // The game normally supplies this Flex base class for embedded binary assets.
    public class ByteArrayAsset extends ByteArray {}
}
package mmo.interfaces.pet.peticon {
    public class PetIconContainer { public static function isBitMapType(id:int):Boolean{return false;} }
}
package mmo.loader.scheduler {
    import flash.system.ApplicationDomain;
    public class SubDomain {
        private static var res:Object = {applicationDomain:new ApplicationDomain(ApplicationDomain.currentDomain)};
        public static function getInstance():Object{return {getPetViewDomain:function():Object{return res;}};}
    }
}
package mmo.loader.scheduler { public class ResPriority { public static const PRELOAD:int=2; } }
package mmo.loader.scheduler {
    public class ResParams {
        public static function create(url:String,done:Function,d:Object,p:int,n:*,err:Function):Object {
            return {url:url,done:done,d:d,err:err};
        }
    }
}
package mmo.loader.scheduler {
    import flash.display.Loader;
    import flash.events.Event;
    import flash.events.IOErrorEvent;
    import flash.net.URLRequest;
    import flash.system.LoaderContext;
    import flash.utils.setTimeout;
    public class ResScheduler {
        public static var requests:int;
        private static var loader:Loader;
        public static function getRes(p:Object):void {
            var part:int=requests++;
            var path:String="peticon/movie/type0/petmovie6044/petmovie6044_"+part;
            if(p.url!=path) {trace("unexpected path",p.url);p.err();return;}
            loader=new Loader();
            loader.contentLoaderInfo.addEventListener(Event.COMPLETE,function(e:Event):void {
                // Combined virtual time exceeds the former twenty-second total deadline.
                setTimeout(p.done,6500);
            });
            loader.contentLoaderInfo.addEventListener(IOErrorEvent.IO_ERROR,function(e:Event):void {
                trace("local read failed",part);p.err();
            });
            loader.load(new URLRequest("part"+part+".swf"),new LoaderContext(false,p.d.applicationDomain));
        }
        public static function cancel(url:String,callback:Function):void {}
    }
}
