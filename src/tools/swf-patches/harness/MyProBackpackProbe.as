package {
    import flash.display.MovieClip;
    import flash.events.Event;
    import flash.utils.getDefinitionByName;
    import probe.ModelHost;
    import mmo.loader.scheduler.ResScheduler;
    import mmo.common.bitmapmovie.BitmapMovieClipParser;
    public class MyProBackpackProbe extends MovieClip {
        private var resolver:Object;
        private var frame:int;
        private function read(target:int):int { ModelHost.target=target; return resolver.resolve(1); }
        public function MyProBackpackProbe() {
            resolver=getDefinitionByName("aola.tools.MyProBackpackMorph");
            trace("official",read(1));
            trace("ready legacy",read(2));
            trace("wrong class",read(3));
            trace("reason",ModelHost.why);
            trace("ready movie",read(4400));
            trace("warm cache",read(4400),BitmapMovieClipParser.parses);
            trace("cold partial",read(4401),read(4401));
            trace("while pending",ModelHost.why,resolver.resolve(99));
            addEventListener(Event.ENTER_FRAME,step);
        }
        private function step(event:Event):void {
            frame++;
            if (frame==10) {
                trace("prepared",ResScheduler.urls.join(","),read(4401));
                trace("cold new",read(4402));
                ModelHost.binding++;
            }
            if (frame==18) {
                trace("stale skips",ResScheduler.urls.length);
                trace("fallback",read(999),ModelHost.why,read(10000),ModelHost.why);
                ModelHost.available=false;
                trace("host off",read(2));
                removeEventListener(Event.ENTER_FRAME,step);
            }
        }
    }
}
package probe {
    public class ModelHost {
        public static var target:int, binding:int=1;
        public static var why:String;
        public static var available:Boolean=true;
        public static function call(name:String,...args):* {
            if(name=="aolaMyProBinding")return binding;
            if(name=="aolaMyProModelCandidate")return args[1]==99?99:target;
            if(name=="aolaMyProModelUse"){why=args[5];return args[4]?args[2]:args[1];}
            if(name=="aolaMyProModelProgress")return true;
            throw new Error(name);
        }
    }
}
package mmo.interfaces {
    public class ServiceContainer {
        public static var instance:Object={getService:function(type:*):Object{return {
            getOperationPM:function(id:int):*{return id==999?null:{pmData:{}};},
            customData:{pmDataMap:{getPM:function(id:int):*{return {};}}}
        };}};
    }
}
package mmo.interfaces.pet.pm.services { public interface IPMService {} }
package mmo.loader.scheduler {
    import flash.system.ApplicationDomain;
    public class SubDomain {
        private static var dom:Object={applicationDomain:ApplicationDomain.currentDomain};
        public static function getInstance():Object{return {getPetViewDomain:function():Object{return dom;}};}
    }
}
package mmo.loader.scheduler {public class ResPriority {public static const PRELOAD:int=2;}}
package mmo.loader.scheduler {public class ResParams {public static function create(url:String,done:Function,d:Object,p:int,n:*,err:Function):Object{return {url:url,done:done};}}}
package mmo.loader.scheduler {
    public class ResScheduler {
        public static var urls:Array=[];
        public static function getRes(p:Object):void{urls.push(p.url);p.done();}
        public static function cancel(url:String,f:Function):void{}
    }
}
package mmo.interfaces.pet.peticon {public class PetIconContainer {public static function isBitMapType(id:int):Boolean{return false;}}}
package mmo.interfaces.pet.peticon {public class PetIconType {public static const BIT_MAP:int=4,LARGE:int=2;}}
package mmo.interfaces.pet.peticon {
    public class PetIconConfig {
        public static const MOVIE_ICON_SPLIT_FILE_NUM:int=4;
        public static function isNewMovieIcon(id:int,type:int):Boolean{return id>=4400;}
        public static function getMovieIconConfigClassName(id:int,type:int,a:int):String{return "probe.ModelConfig";}
        public static function getMovieIconClassName(id:int,type:int,a:int):String{return "probe.Frame";}
        public static function getMovieIconRes(id:int,t:int,p:int,a:int):String{return "piece"+p;}
    }
}
package mmo.common.bitmapmovie {
    import mmo.loader.scheduler.ResScheduler;
    import probe.ModelHost;
    public class BitmapMovieClipParser {
        public static var parses:int;
        public static function getConfigData(n:String,d:*):String{return "frames";}
        public static function parseConfig(s:String):Array { parses++;
            return [{bitmapId:(ModelHost.target==4400 || (ModelHost.target==4401 && ResScheduler.urls.length==4))?0:9,x:0,y:0}];
        }
    }
}
package probe {public class ModelConfig {}}
package probe {import flash.display.BitmapData;public class Frame_0 extends BitmapData {public function Frame_0(){super(1,1);}}}
package mmo.pet {import flash.display.Sprite;public class peticon2_L extends Sprite {}}
package mmo.pet {public class peticon3_L {}}
