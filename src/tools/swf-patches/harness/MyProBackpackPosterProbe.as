package {
    import flash.display.MovieClip;
    import flash.display.Sprite;
    import flash.display.DisplayObject;
    import flash.utils.getDefinitionByName;
    import flash.utils.getQualifiedClassName;
    import probe.ModelHost;
    import mmo.loader.scheduler.ResScheduler;
    public class MyProBackpackPosterProbe extends MovieClip {
        private var morph:Object;
        private var layer:Sprite=new Sprite(), panel:Sprite=new Sprite();
        private var pdi:Object={viewId:17,specialSkinId:55,useDIYBgId:7};
        private var clears:int;
        private var service:Object={
            getPetSkinInfoBySkinId:function(id:int):Object {
                if(id==21) return null;
                var res:Object={clsName:id==20?"probe.ModelHost":(id==22?"probe.Cold":"probe.Main"),resPath:"cold"};
                if(id==23) res={clsName:"mmo.petskin.view.Background1029",resPath:"fresh"};
                return {skinId:id,hasBackground:true,backgroundRes:res,isSwitchPackageBG:id==19,
                    isSwitchPackageBGNormalSize:false,packageBackgroundRes:{clsName:"probe.Wide",resPath:"wide"}};
            },
            isInMultiPetPackageState:function(id:int):Boolean{return id==19;}
        };
        private function clear():void {
            clears++;
            while(layer.numChildren) layer.removeChildAt(0);
            while(panel.numChildren) panel.removeChildAt(0);
        }
        private function run(id:int):void {
            morph.cancelPoster(layer);
            while(layer.numChildren) layer.removeChildAt(0);
            var original:Sprite=new Sprite(); original.name="original"; layer.addChild(original);
            ModelHost.target=id;
            morph.poster(pdi,service,layer,panel,clear);
        }
        private function result():String {
            var group:Sprite=layer.getChildAt(0) as Sprite;
            return group.numChildren?group.getChildAt(0).name:group.name;
        }
        public function MyProBackpackPosterProbe() {
            morph=getDefinitionByName("aola.tools.MyProBackpackMorph");
            run(0); trace("official",result(),clears);
            run(18); trace("warm",result(),Sprite(layer.getChildAt(0)).mouseChildren,clears,pdi.viewId,pdi.specialSkinId,pdi.useDIYBgId);
            run(17); trace("same",result(),clears);
            run(19); var wide:Sprite=panel.getChildAt(0) as Sprite; trace("wide",wide.getChildAt(0).name,wide.x,wide.name,result());
            run(20); trace("invalid",result(),ModelHost.reason);
            run(21); trace("missing",result(),ModelHost.reason);
            run(22); trace("pending",result(),ModelHost.reason,ResScheduler.request.url);
            ResScheduler.request.fail(); trace("failed",result(),ModelHost.reason);
            ResScheduler.cancelledAfterError=0;
            run(22); var late:Function=ResScheduler.request.done; morph.cancelPoster(layer); late();
            trace("cancelled",result(),ResScheduler.cancelledAfterError);
            run(22); ModelHost.target=24; ResScheduler.request.done();
            trace("stale",result(),ModelHost.staleSelected);
            ModelHost.available=false; run(18); trace("host off",result());
            ModelHost.available=true;
            ResScheduler.complete=function():void {
                var art:DisplayObject=Sprite(layer.getChildAt(0)).getChildAt(0);
                trace("loaded",getQualifiedClassName(art),art.width>0,ModelHost.target);
            };
            run(23); morph.checkPoster(layer,pdi); trace("cold",result(),"pending");
        }
    }
}
package probe {
    public class ModelHost {
        public static var target:int, staleSelected:int;
        public static var reason:String, available:Boolean=true;
        public static function call(name:String,...args):* {
            if(name=="aolaMyProBinding") return 1;
            if(name=="aolaMyProModelCandidate") return target;
            if(name=="aolaMyProModelUse"){
                reason=args[5]; if(args[4] && args[2]!=target) staleSelected++;
                return args[4]?args[2]:0;
            }
        }
    }
}
package probe {
    import flash.display.Sprite;
    import mmo.interfaces.IDataInitable;
    public class Main extends Sprite implements IDataInitable {
        public function init(data:Object):void { name=data.canInteract?"interactive":"Main"; }
    }
}
package mmo.interfaces { public interface IDataInitable {function init(data:Object):void;} }
package probe {import flash.display.Sprite;public class Main_2 extends Sprite {public function Main_2(){name="Variant";}}}
package probe {import flash.display.Sprite;public class Wide extends Sprite {public function Wide(){name="Wide";}}}
package mmo.loader.scheduler { public class ResDomain {public var domain:Object;public function ResDomain(d:Object){domain=d;}} }
package mmo.loader.scheduler { public class ResPriority {public static const PRELOAD:int=1;} }
package mmo.loader.scheduler {
    public class ResParams {
        public static function create(url:String,done:Function,d:Object,p:int,n:*,fail:Function):Object {
            return {url:url,done:done,fail:fail,domain:d.domain};
        }
    }
}
package mmo.loader.scheduler {
    import flash.display.Loader;
    import flash.events.Event;
    import flash.net.URLRequest;
    import flash.system.LoaderContext;
    public class ResScheduler {
        public static var request:Object, cancelledAfterError:int,complete:Function;
        private static var loader:Loader;
        public static function getRes(r:Object):void {
            request=r;
            if(r.url!="fresh") return;
            loader=new Loader();
            loader.contentLoaderInfo.addEventListener(Event.COMPLETE,function(e:Event):void { r.done(); complete(); });
            loader.load(new URLRequest("art.swf"),new LoaderContext(false,r.domain));
        }
        public static function cancel(url:String,done:Function):void {
            if(request==null || request.done!==done) throw new Error("wrong request");
            request=null; cancelledAfterError++;
        }
    }
}
