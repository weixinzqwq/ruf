package {
    import flash.display.Sprite;
    import flash.events.Event;
    import flash.utils.getDefinitionByName;
    import probe.external.ExternalInterface;
    import mmo.framework.comm.PetClient;
    import mmo.framework.comm.material.HKClient;
    import mmo.framework.comm.material.ToteClient;
    import mmo.framework.comm.material.CrystalKeyClient;
    public class MyProLineupProbe extends Sprite {
        public function MyProLineupProbe() { addEventListener(Event.ENTER_FRAME,run); }
        private function run(e:Event):void {
            removeEventListener(Event.ENTER_FRAME,run);
            var helper:Object=getDefinitionByName("aola.tools.MyProLineup");
            var pet:PetClient=PetClient.instance;
            helper.bind(this,pet.rows());
            var read:Function=function():Object{return JSON.parse(ExternalInterface.callbacks.aolaLineupRead());};
            var restore:Function=function(w:Object,k:String):Object{return JSON.parse(ExternalInterface.callbacks.aolaLineupRestore(JSON.stringify(w),k));};
            trace("initial",read().current.ownerPlayerId,read().current.pets.length,read().sections.join(","),read().canSave,pet.reads);
            var frozen:Object=JSON.parse(ExternalInterface.callbacks.aolaLineupCapture("test"));
            trace("capture",frozen.stage,frozen.captureId,pet.reads);
            var snap:Object=JSON.parse(ExternalInterface.callbacks.aolaLineupRead("test"));
            trace("verified snapshot",snap.captured.pets[0].petId,read().captured==null);
            var w:Object=read().current;
            w.pets.reverse();
            trace("order",restore(w,"order").stage,pet.ids.join(","),pet.writes);
            trace("same",restore(w,"order").stage,pet.writes);
            w.pets[0].soulCards=[11,0,0,0,0];
            trace("cards",restore(w,"cards").stage,HKClient.writes,pet.cards);
            w.pets[0].soulArtifacts=[{instanceId:98,configId:37},null];
            trace("artifacts",restore(w,"artifacts").stage,ToteClient.writes,pet.art);
            w.pets[0].crystalKeys=[33,0];
            trace("keys",restore(w,"keys").stage,CrystalKeyClient.writes,pet.key);
            w.pets[0].crystalKeys=[55,0];
            trace("unbound",restore(w,"keys").stage,CrystalKeyClient.writes);
            w.pets[0].soulCards=[44,0,0,0,0];
            trace("unowned",restore(w,"cards").stage,HKClient.writes);
            w.pets[0].soulArtifacts=[{instanceId:99,configId:37},null];
            trace("unowned artifact",restore(w,"artifacts").stage,ToteClient.writes);
            w.pets[0].crystalKeys=[0,0];
            trace("clear key",restore(w,"keys").stage,CrystalKeyClient.writes);
            w.ownerPlayerId=999;
            trace("owner",restore(w,"order").stage,pet.writes);
            w.ownerPlayerId=700;
            w.pets[0].petId=333;
            trace("storage other owner",restore(w,"order").stage,pet.writes);
            pet.otherOwner=700;
            trace("storage same owner",restore(w,"order").stage,pet.ids.join(","),pet.writes);
            w=read().current;
            w.pets[0].crystalKeys=[33,0];
            CrystalKeyClient.change=false;
            trace("false ack",restore(w,"keys").stage,CrystalKeyClient.writes);
            w.pets[0].soulCards=[11,0,0,0,0];
            HKClient.hold=true;
            trace("pending",restore(w,"cards").stage);
            ExternalInterface.allowed=false;
            HKClient.finish();
            ExternalInterface.allowed=true;
            trace("battle interrupts",read().stage,HKClient.writes);
            helper.unbind(this);
            trace("closed",ExternalInterface.callbacks.aolaLineupRead==null,ExternalInterface.callbacks.aolaLineupCapture==null);
        }
    }
}
package probe.external {
    public class ExternalInterface {
        public static var available:Boolean=true;
        public static var allowed:Boolean=true;
        public static var callbacks:Object={};
        public static function addCallback(name:String,f:Function):void {callbacks[name]=f;}
        public static function call(name:String):* {return name=="aolaLineupEditingAllowed" && allowed;}
    }
}
package probe {
    import flash.events.Event;
    public class Response extends Event {
        public var params:Object;
        public function Response(type:String,data:Object) {super(type);params=data;}
    }
}
package mmo.framework.comm {
    import flash.events.EventDispatcher;
    public class SocketClient {public static var instance:EventDispatcher=new EventDispatcher();}
}
package mmo.framework.comm {
    import probe.Response;
    public class PetClient {
        public static const RSPN_VIEW_ALL_PM:String="bag", RSPN_VIEW_SINGLE_PM:String="pet", RSPN_SET_PM_BATTLE_LIST:String="order";
        public static var instance:PetClient=new PetClient();
        public var ids:Array=[111,222], reads:int=0,writes:int=0, key:int=0,cards:int=0,art:String="",otherOwner:int=800;
        public function rows():Array {
            var out:Array=[];
            for each(var i:int in ids) {
                var pcp:String=i==222 ? '<p soc="0:'+cards+'#0:0#0:0#0:0#0:0" tt="'+art+'|77;22;x" ctk="'+key+'_0"/>' :
                    i==333 ? '<p soc="0:0#0:0#0:0#0:0#0:0" ctk="0_0"/>' : '<p/>';
                out.push({pmSView:{i:i,r:1,m:700,nm:"pet",pcp:pcp}});
            }
            return out;
        }
        public function viewAllPM():void {
            ++reads; SocketClient.instance.dispatchEvent(new Response("bag",{pms:ids.join("#"),pmList:rows()}));
        }
        public function viewSinglePM(id:int):void {SocketClient.instance.dispatchEvent(new Response("pet",{pm:{pmPView:{i:id,r:1,m:otherOwner}}}));}
        public function setPmBattleList(idsText:String):void {
            ++writes;ids=idsText.split("#"); SocketClient.instance.dispatchEvent(new Response("order",{pms:idsText}));
        }
    }
}
package mmo.framework.comm.material {
    import mmo.framework.comm.PetClient;
    public class HKClient {
        public static var writes:int=0, hold:Boolean=false, callback:Function;
        public static function getMyHKData(cb:Function):void {callback=cb;if(!hold) finish();}
        public static function finish():void {callback({cl:[{ci:11,n:1}]});}
        public static function saveCard(i:int,arr:Array,bk:int,cb:Function,tf:Boolean):void {
            ++writes;PetClient.instance.cards=arr[0];cb({r:1});
        }
    }
}
package mmo.framework.comm.material {
    import mmo.framework.comm.PetClient;
    public class ToteClient {
        public static var writes:int=0;
        public static function getMyData(cb:Function):void{cb({utl:[{ui:98,ti:37}]});}
        public static function putOn(id:int,ui:int,cb:Function):void{++writes;PetClient.instance.art="98;37;x";cb({r:1});}
    }
}
package mmo.framework.comm.material {
    import mmo.framework.comm.PetClient;
    public class CrystalKeyClient {
        public static var writes:int=0, change:Boolean=true;
        public static function getBindData(id:int,cb:Function):void{cb({r:1,ids:[33]});}
        public static function equipBindKey(id:int,slot:int,key:int,cb:Function):void{
            ++writes;if(change)PetClient.instance.key=key;cb({r:1});
        }
    }
}
