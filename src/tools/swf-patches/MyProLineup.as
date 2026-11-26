package aola.tools {
    import flash.display.DisplayObject;
    import flash.events.Event;
    import flash.external.ExternalInterface;
    import flash.utils.clearTimeout;
    import flash.utils.getDefinitionByName;
    import flash.utils.setTimeout;

    public final class MyProLineup {
        private static var panel:DisplayObject;
        private static var bag:Object;
        private static var plan:Object;
        private static var section:String;
        private static var stage:String = "ready";
        private static var message:String = "";
        private static var serial:uint;
        private static var timer:uint;
        private static var listener:Function;
        private static var eventName:String;
        private static var emitter:Object;
        private static var captureId:String="";
        private static var captured:Object;

        public static function bind(owner:DisplayObject, rows:Array):void {
            if (!ExternalInterface.available || !owner) return;
            if (panel !== owner) {
                unbind(panel);
                panel = owner;
                panel.addEventListener(Event.REMOVED_FROM_STAGE, removed);
                try {
                    ExternalInterface.addCallback("aolaLineupRead", readState);
                    ExternalInterface.addCallback("aolaLineupCapture", capture);
                    ExternalInterface.addCallback("aolaLineupRestore", restore);
                } catch (ignored:Error) { unbind(owner); return; }
            }
            try {
                var latest:Object = parseBag(rows);
                if (plan && latest.ownerPlayerId != plan.ownerPlayerId) finish("failed","当前背包所属角色已变更");
                bag = latest;
            } catch (e:Error) { bag = null; finish("failed",e.message); }
        }
        private static function removed(e:Event):void { if (e.target === panel) unbind(panel); }
        public static function unbind(owner:DisplayObject):void {
            if (!panel || owner !== panel) return;
            finish("ready","");
            panel.removeEventListener(Event.REMOVED_FROM_STAGE, removed);
            panel = null; bag = null; captured=null; captureId="";
            try {
                ExternalInterface.addCallback("aolaLineupRead",null);
                ExternalInterface.addCallback("aolaLineupCapture",null);
                ExternalInterface.addCallback("aolaLineupRestore",null);
            } catch (ignored:Error) {}
        }
        private static function petClient():Object { return getDefinitionByName("mmo.framework.comm.PetClient").instance; }
        private static function materials(kind:String):Object {
            return getDefinitionByName("mmo.framework.comm.material." +
                (kind == "cards" ? "HKClient" : kind == "artifacts" ? "ToteClient" : "CrystalKeyClient"));
        }
        private static function permitted():Boolean {
            return panel && panel.stage && bag && ExternalInterface.call("aolaLineupEditingAllowed") === true;
        }
        private static function available():Array {
            var result:Array = [];
            try { if (petClient().setPmBattleList is Function && petClient().viewSinglePM is Function) result.push("order"); } catch (ignored:Error) {}
            try { if (materials("cards").getMyHKData is Function && materials("cards").saveCard is Function) result.push("cards"); } catch (ignored2:Error) {}
            try { if (materials("artifacts").getMyData is Function && materials("artifacts").putOn is Function) result.push("artifacts"); } catch (ignored3:Error) {}
            try { if (materials("keys").getBindData is Function && materials("keys").equipBindKey is Function) result.push("keys"); } catch (ignored4:Error) {}
            return result;
        }
        private static function readState(token:String=null):String {
            var canEdit:Boolean = permitted();
            var result:Object={current:bag,stage:canEdit ? stage : "unavailable",message:message,captureId:captureId,
                sections:canEdit ? available() : [],canSave:canEdit && !plan};
            if(token && token==captureId && canEdit && stage=="ready") result.captured=captured;
            return JSON.stringify(result);
        }
        private static function id(value:*, min:int=0):Number {
            var n:Number=Number(value);
            if (value == null || String(value).length == 0 || n < min || n > 4294967295 || n != Math.floor(n) || !isFinite(n))
                throw new Error("背包资料含无效编号");
            return n;
        }
        private static function configList(value:String, separator:String, count:int, kind:String):Array {
            var out:Array = [];
            var fields:Array = value.split(separator);
            if (fields.length != count) throw new Error("背包配置槽数不完整");
            for each (var field:String in fields) {
                var parts:Array;
                if (kind == "cards") {
                    parts = field.split(":");
                    if (parts.length != 2) throw new Error("魂卡资料不完整");
                    out.push(id(parts[1]));
                } else if (kind == "artifacts") {
                    if (!field) out.push(null);
                    else {
                        parts = field.split(";");
                        if (parts.length < 3) throw new Error("魂器资料不完整");
                        out.push({instanceId:id(parts[0],1),configId:id(parts[1],1)});
                    }
                } else out.push(id(field));
            }
            return out;
        }
        private static function parseBag(rows:Array):Object {
            if (!rows || !rows.length || rows.length>6) throw new Error("等待完整的 1 至 6 只亚比背包");
            var owner:Number=0, pets:Array=[], seen:Object={};
            for each (var row:Object in rows) {
                var v:Object = row.pmPView || row.pmSView;
                var uid:Number=id(v.m,1), petId:Number=id(v.i,1);
                if ((owner && owner!=uid) || seen[petId]) throw new Error("背包所属角色或实例不一致");
                owner=uid; seen[petId]=true;
                if (!(v.pcp is String)) throw new Error("背包缺少配置资料");
                var xml:XML=new XML(v.pcp);
                if (xml.localName()!="p") throw new Error("背包配置格式无效");
                pets.push({petId:petId,raceId:id(v.r,1),name:String(v.nm||""),
                    soulCards:xml.@soc.length()?configList(String(xml.@soc),"#",5,"cards"):null,
                    soulArtifacts:xml.@tt.length()?configList(String(xml.@tt),"|",2,"artifacts"):null,
                    crystalKeys:xml.@ctk.length()?configList(String(xml.@ctk),"_",2,"keys"):null});
            }
            return {ownerPlayerId:owner,pets:pets};
        }
        private static function capture(token:String):String {
            if(plan || !permitted()) return readState();
            captureId=token; captured=null;
            plan={ownerPlayerId:bag.ownerPlayerId};stage="saving";message="正在核对两次背包读数";
            try {
                refresh(function():void {
                    var frozen:String=JSON.stringify(bag);
                    refresh(function():void {
                        if(JSON.stringify(bag)!=frozen) throw new Error("两次回读之间背包发生变化");
                        captured=bag;
                        finish("ready","两次服务器背包回读一致");
                    });
                });
            } catch(e:Error) { finish("failed",e.message); }
            return readState();
        }
        private static function lookup(petId:Number):Object {
            for each (var pet:Object in bag.pets) if (pet.petId==petId) return pet;
            return null;
        }
        private static function order(pets:Array):String {
            var ids:Array=[];
            for each (var pet:Object in pets) ids.push(pet.petId);
            return ids.join("#");
        }
        private static function disarm():void {
            ++serial;
            if (timer) clearTimeout(timer);
            timer=0;
            if (listener) emitter.removeEventListener(eventName,listener);
            listener=null; emitter=null;
        }
        private static function finish(next:String, text:String):void {
            disarm(); plan=null; stage=next; message=text;
        }
        private static function guarded(next:Function):Function {
            disarm();
            var token:uint=serial;
            timer=setTimeout(function():void {
                if(token==serial) finish("failed","游戏回读超时，请重新打开背包核对后再试");
            },8000);
            return function(data:Object):void {
                if (serial!=token || !plan) return;
                disarm();
                try {
                    if (!permitted() || bag.ownerPlayerId!=plan.ownerPlayerId) throw new Error("当前角色或战斗状态已变更");
                    next(data);
                } catch(e:Error) { finish("failed",e.message); }
            };
        }
        private static function listen(eventKey:String,next:Function,predicate:Function=null):void {
            var callback:Function=guarded(next);
            var type:Object=getDefinitionByName("mmo.framework.comm.PetClient");
            emitter=getDefinitionByName("mmo.framework.comm.SocketClient").instance;
            eventName=String(type[eventKey]);
            listener=function(e:Object):void {
                var data:Object=e.params;
                if (!predicate || predicate(data)) callback(data);
            };
            emitter.addEventListener(eventName,listener);
        }
        private static function refresh(next:Function):void {
            listen("RSPN_VIEW_ALL_PM",function(data:Object):void {
                var ordered:Array=[], indexed:Object={};
                for each(var row:Object in data.pmList) {
                    var v:Object=row.pmPView || row.pmSView;
                    indexed[id(v.i,1)]=row;
                }
                for each(var key:String in String(data.pms).split("#")) ordered.push(indexed[key]);
                if (ordered.length!=data.pmList.length) throw new Error("背包排序与服务器的实例资料不符");
                bag=parseBag(ordered);
                if (bag.ownerPlayerId!=plan.ownerPlayerId) throw new Error("方案所属角色与当前账号不符");
                next();
            });
            petClient().viewAllPM();
        }
        private static function restore(raw:String,part:String):String {
            if (plan) return readState();
            try {
                if (!permitted() || available().indexOf(part)<0) throw new Error("请在战外打开自己的亚比背包");
                var preset:Object=JSON.parse(raw);
                if (preset.ownerPlayerId!=bag.ownerPlayerId || !(preset.pets is Array) || !preset.pets.length || preset.pets.length>6)
                    throw new Error("方案所属角色或名单无效");
                captured=null; captureId=""; steps=0; plan=preset; section=part; stage="restoring"; message="正在核对背包和实例归属";
                refresh(function():void { verifyPets(0); });
            } catch(e:Error) { finish("failed",e.message); }
            return readState();
        }
        private static function verifyPets(index:int):void {
            if(index==plan.pets.length) {
                if(section=="order") restoreOrder(); else advance();
                return;
            }
            var wanted:Object=plan.pets[index], here:Object=lookup(wanted.petId);
            if(here) {
                if(here.raceId!=wanted.raceId) throw new Error("亚比实例与品种不符");
                verifyPets(index+1); return;
            }
            if(section!="order") throw new Error("请先恢复背包顺序，再恢复配置");
            listen("RSPN_VIEW_SINGLE_PM",function(data:Object):void {
                var v:Object=data.pm.pmPView || data.pm.pmSView;
                if(id(v.m,1)!=plan.ownerPlayerId || id(v.r,1)!=wanted.raceId) throw new Error("无法确认目标亚比归属和品种");
                verifyPets(index+1);
            },function(data:Object):Boolean {
                if(!data || !data.pm) return false;
                var v:Object=data.pm.pmPView || data.pm.pmSView;
                return v && v.i==wanted.petId;
            });
            petClient().viewSinglePM(wanted.petId);
        }
        private static function restoreOrder():void {
            var desired:String=order(plan.pets);
            if(order(bag.pets)==desired) { finish("ready","服务器回读确认阵容顺序一致"); return; }
            message="正在恢复背包并等待服务器回读";
            listen("RSPN_SET_PM_BATTLE_LIST",function(data:Object):void {
                if(data.pms!=desired) throw new Error("服务器返回的顺序与方案不同");
                refresh(function():void {
                    if(order(bag.pets)!=desired) throw new Error("背包回读顺序与方案不同");
                    finish("ready","服务器回读确认阵容顺序一致");
                });
            });
            petClient().setPmBattleList(desired);
        }
        private static function field():String {
            return section=="cards"?"soulCards":section=="artifacts"?"soulArtifacts":"crystalKeys";
        }
        private static function equalItem(a:*,b:*):Boolean {
            return section=="artifacts" ? !a || (b && a.instanceId==b.instanceId && a.configId==b.configId) : a==b;
        }
        private static var steps:int;
        private static function advance():void {
            var hadField:Boolean=false;
            for each(var wanted:Object in plan.pets) {
                var here:Object=lookup(wanted.petId), key:String=field();
                if(!here || here.raceId!=wanted.raceId) throw new Error("恢复期间背包发生变化");
                var goal:Array=wanted[key];
                if(!goal) continue;
                hadField=true;
                var actual:Array=here[key];
                if(!actual || actual.length!=goal.length) throw new Error("该亚比缺少相应配置资料");
                for(var slot:int=0;slot<goal.length;slot++) {
                    if(equalItem(goal[slot],actual[slot])) continue;
                    if(++steps>30) throw new Error("配置持续变化，请重新读取背包");
                    change(wanted,here,slot); return;
                }
            }
            finish("ready",hadField?"整套阵容的所选配置已通过服务器回读":"该方案没有此项配置");
        }
        private static function change(wanted:Object,here:Object,slot:int):void {
            var petId:int=wanted.petId, key:String=field(), goal:Array=wanted[key];
            message="正在恢复 "+(wanted.name||petId)+"，等待服务器确认";
            var accepted:Function=function(reply:Object):void {
                if(!reply || reply.r!=1) throw new Error("游戏未确认本次配置修改，请核对已完成的项目");
                refresh(function():void {
                    var pet:Object=lookup(petId);
                    if(!pet || !pet[key] || !equalItem(goal[slot],pet[key][slot])) throw new Error("配置修改后的背包回读不一致");
                    advance();
                });
            };
            var api:Object=materials(section);
            var stillCurrent:Function=function():void {
                var recent:Object=lookup(petId);
                if(!recent || recent.raceId!=wanted.raceId || JSON.stringify(recent[key])!=JSON.stringify(here[key]))
                    throw new Error("等待游戏确认期间该亚比配置发生变化");
            };
            if(section=="keys") {
                if(!goal[slot]) throw new Error("游戏未提供已核实的单槽晶钥卸下操作");
                api.getBindData(petId,guarded(function(data:Object):void {
                    if(!data || data.r!=1 || !(data.ids is Array) || data.ids.indexOf(goal[slot])<0) throw new Error("目标晶钥尚未绑定到该亚比");
                    stillCurrent();
                    api.equipBindKey(petId,slot,goal[slot],guarded(accepted));
                }));
            } else if(section=="artifacts") {
                api.getMyData(guarded(function(data:Object):void {
                    var item:Object=goal[slot];
                    if(data && data.utl is Array) for each(var entry:Object in data.utl) {
                        if(entry.ui==item.instanceId && entry.ti==item.configId) {
                            stillCurrent();
                            api.putOn(petId,item.instanceId,guarded(accepted)); return;
                        }
                    }
                    throw new Error("在自己的魂器库存中未找到目标实例");
                }));
            } else {
                api.getMyHKData(guarded(function(data:Object):void {
                    if(!data || !(data.cl is Array)) throw new Error("游戏未返回自己的魂卡库存");
                    var owned:Object={};
                    for each(var inventory:Object in data.cl) owned[id(inventory.ci,1)] = id(inventory.n);
                    for each(var already:int in here.soulCards) if(already) owned[already]=int(owned[already])+1;
                    for each(var card:int in goal) if(card && (!owned[card] || --owned[card]<0))
                        throw new Error("自己的可用魂卡数量不足");
                    stillCurrent();
                    api.saveCard(petId,goal,0,guarded(accepted),false);
                }));
            }
        }
    }
}
