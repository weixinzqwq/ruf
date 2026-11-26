package aola.tools {
    import flash.display.BitmapData;
    import flash.display.DisplayObject;
    import flash.display.Sprite;
    import flash.external.ExternalInterface;
    import flash.system.ApplicationDomain;
    import flash.utils.getDefinitionByName;
    import flash.utils.Dictionary;
    import flash.utils.setTimeout;
    import flash.utils.clearTimeout;
    import flash.utils.getTimer;
    import mmo.interfaces.IDataInitable;

    public final class MyProBackpackMorph {
        private static var active:Object;
        private static var cooldown:Dictionary = new Dictionary(true);
        private static var verified:Dictionary = new Dictionary(true);
        private static var reason:String;
        private static var posterJob:Object;

        public static function cancelPoster(layer:Sprite):void {
            var job:Object = posterJob;
            if (!job || job.layer !== layer) return;
            posterJob = null;
            clearTimeout(job.timer);
            if (job.done != null) {
                try { job.scheduler.cancel(job.url, job.done); } catch (ignored:Error) {}
            }
        }

        public static function checkPoster(layer:Sprite,pdi:Object):void {
            if (!posterJob || posterJob.layer !== layer) return;
            if (!pdi || pdi.id != posterJob.pet || pdi.viewId != posterJob.source ||
                pdi.specialSkinId != posterJob.skin || pdi.useDIYBgId != posterJob.diy) cancelPoster(layer);
        }

        // changeSkins calls this after drawing the official background. Only retire
        // that background once every configured visual can actually be instantiated.
        public static function poster(pdi:Object, service:Object, layer:Sprite, panel:Sprite, clear:Function):void {
            if (!layer || !panel || !pdi || !ExternalInterface.available) return;
            var job:Object;
            try {
                var source:int = pdi.viewId;
                if (source <= 0) return;
                var target:int = int(ExternalInterface.call("aolaMyProModelCandidate","portrait",source,false));
                if (target <= 0) return;
                if (posterJob) cancelPoster(posterJob.layer);
                var binding:Number = Number(ExternalInterface.call("aolaMyProBinding"));
                if (!(binding > 0)) return;
                job = {source:source,target:target,binding:binding,pdi:pdi,pet:pdi.id,skin:pdi.specialSkinId,diy:pdi.useDIYBgId,layer:layer,panel:panel,clear:clear,
                    assets:[],next:0,timer:0,url:null,done:null,domain:ApplicationDomain.currentDomain};
                posterJob = job;
                var info:Object = service.getPetSkinInfoBySkinId(target % 10000);
                if (info && info.isSwitchPackageBG) {
                    var r:Object = info.packageBackgroundRes;
                    job.assets.push({path:r.resPath,cls:r.clsName,wide:!info.isSwitchPackageBGNormalSize});
                }
                if (info && info.hasBackground) {
                    r = info.backgroundRes;
                    var variant:Boolean = target > 10000 || service.isInMultiPetPackageState(info.skinId);
                    job.assets.push({path:r.resPath,cls:r.clsName + (variant ? "_2" : ""),wide:false});
                }
                if (!job.assets.length) { failPoster(job,"config-missing"); return; }
                loadPoster(job);
            } catch (error:Error) { if (job) failPoster(job,"display-check-error"); }
        }

        private static function currentPoster(job:Object):Boolean {
            return posterJob === job && job.pdi.viewId == job.source &&
                Number(ExternalInterface.call("aolaMyProBinding")) == job.binding &&
                int(ExternalInterface.call("aolaMyProModelCandidate","portrait",job.source,false)) == job.target;
        }
        private static function failPoster(job:Object, why:String):void {
            if (posterJob !== job) return;
            cancelPoster(job.layer);
            try { ExternalInterface.call("aolaMyProModelUse","portrait",job.source,job.target,false,false,why); }
            catch (ignored:Error) {}
        }
        private static function loadPoster(job:Object):void {
            if (posterJob !== job) return;
            try {
                if (!currentPoster(job)) { cancelPoster(job.layer); return; }
                var domain:ApplicationDomain = job.domain;
                while (job.next < job.assets.length) {
                    var asset:Object = job.assets[job.next];
                    if (!asset.path || !asset.cls) { failPoster(job,"invalid-config"); return; }
                    if (!domain.hasDefinition(asset.cls)) {
                        if (asset.requested) { failPoster(job,"frame-missing"); return; }
                        asset.requested = true;
                        var params:Object = getDefinitionByName("mmo.loader.scheduler.ResParams");
                        var priorities:Object = getDefinitionByName("mmo.loader.scheduler.ResPriority");
                        var domainClass:Class = getDefinitionByName("mmo.loader.scheduler.ResDomain") as Class;
                        job.scheduler = getDefinitionByName("mmo.loader.scheduler.ResScheduler");
                        job.url = asset.path;
                        var done:Function = function(...unused):void {
                            if (posterJob !== job || job.done !== done) return;
                            clearTimeout(job.timer);
                            job.done = null;
                            loadPoster(job);
                        };
                        job.done = done;
                        job.timer = setTimeout(function():void { failPoster(job,"preload-timeout"); },20000);
                        ExternalInterface.call("aolaMyProModelUse","portrait",job.source,job.target,false,false,"preload-pending");
                        job.scheduler.getRes(params.create(asset.path,job.done,new domainClass(domain),priorities.PRELOAD,
                            null,function(...unused):void { if (job.done === done) failPoster(job,"preload-error"); }));
                        return;
                    }
                    if (!extendsType(domain,asset.cls,DisplayObject)) { failPoster(job,"class-mismatch"); return; }
                    job.next++;
                }
                // Use the same visual initialization contract as PetSkinDataService.getView.
                // Keep mapped art passive. Game buttons and the selected pet retain their official state.
                for each (asset in job.assets) {
                    var cls:Class = domain.getDefinition(asset.cls) as Class;
                    var visual:DisplayObject = new cls() as DisplayObject;
                    if (visual is IDataInitable) IDataInitable(visual).init({canInteract:false});
                    var wrapper:Sprite = new Sprite();
                    wrapper.mouseEnabled = wrapper.mouseChildren = false;
                    wrapper.addChild(visual);
                    if (asset.wide) { wrapper.name = "skin" + job.target; wrapper.x = -200; }
                    asset.wrapper = wrapper;
                }
                if (!currentPoster(job)) { cancelPoster(job.layer); return; }
                if (int(ExternalInterface.call("aolaMyProModelUse","portrait",job.source,job.target,false,true,"")) != job.target) {
                    cancelPoster(job.layer); return;
                }
                cancelPoster(job.layer);
                job.clear();
                for each (asset in job.assets) (asset.wide ? job.panel : job.layer).addChild(asset.wrapper);
            } catch (error:Error) { failPoster(job,"display-check-error"); }
        }
        public static function resolve(source:int):int {
            if (source <= 0 || !ExternalInterface.available) return source;
            try {
                var target:int = int(ExternalInterface.call("aolaMyProModelCandidate", "backpack", source, false));
                if (active && active.source == source && active.id != target) finish(active);
                if (target <= 0 || target == source) return source;
                var ready:Boolean = false;
                reason = "display-check-error";
                try { ready = available(source,target); } catch (invalid:Error) { reason = "display-check-error"; }
                var selected:int = int(ExternalInterface.call("aolaMyProModelUse", "backpack", source, target, false, ready, ready ? "" : reason));
                return ready && selected == target ? target : source;
            } catch (error:Error) {
                return source;
            }
        }

        // Use the same domain and keys as PetIconContainer.startLoad and the two
        // LARGE/BIT_MAP loaders selected by PetBigIconDetail.afterOtherOperate.
        private static function available(source:int,id:int):Boolean {
            // PetIconContainer normalizes view IDs with % 10000. Do not alias namespaces.
            if (id >= 10000) { reason = "unsupported-id"; return false; }
            var services:Object = getDefinitionByName("mmo.interfaces.ServiceContainer");
            var pm:Object = services.instance.getService(getDefinitionByName("mmo.interfaces.pet.pm.services.IPMService"));
            var operation:Object = pm.getOperationPM(id);
            if (!operation || !operation.pmData || !pm.customData.pmDataMap.getPM(id)) { reason = "pm-data-missing"; return false; }
            var sub:Object = getDefinitionByName("mmo.loader.scheduler.SubDomain");
            var res:Object = sub.getInstance().getPetViewDomain();
            var domain:ApplicationDomain = res.applicationDomain;
            if (active && active.domain !== domain) finish(active);
            var icons:Object = getDefinitionByName("mmo.interfaces.pet.peticon.PetIconContainer");
            var types:Object = getDefinitionByName("mmo.interfaces.pet.peticon.PetIconType");
            var bitmap:Boolean = icons.isBitMapType(id);
            var type:int = bitmap ? types.BIT_MAP : types.LARGE;
            var key:String=id+"."+type;
            var verifiedKeys:Array=verified[domain] as Array;
            if (verifiedKeys && verifiedKeys.indexOf(key)>=0) return true;
            var config:Object = getDefinitionByName("mmo.interfaces.pet.peticon.PetIconConfig");
            if (!config.isNewMovieIcon(id, type)) {
                var legacy:String = "mmo.pet.peticon" + id + (bitmap ? "_B" : "_L");
                if (extendsType(domain, legacy, Sprite)) return remember(domain,key);
                if (domain.hasDefinition(legacy)) { reason = "class-mismatch"; return false; }
                reason = "config-missing";
                prepare(res, source,id, ["peticon/peticon" + id]);
                return false;
            }
            var prefix:String = config.getMovieIconClassName(id, type, -1);
            var configClass:String = config.getMovieIconConfigClassName(id, type, -1);
            if (!domain.hasDefinition(configClass)) { reason = "config-missing"; prepareMovie(res,source,id,type,config); return false; }
            reason = "invalid-config";
            var parser:Object = getDefinitionByName("mmo.common.bitmapmovie.BitmapMovieClipParser");
            var text:String = parser.getConfigData(configClass, domain);
            if (!text || text.length > 65536) return false;
            var frames:Array = parser.parseConfig(text);
            if (!frames.length || frames.length > 1200) return false;
            var checked:Object = {};
            for each (var frame:Object in frames) {
                var n:Number = frame.bitmapId;
                if (!isFinite(n) || n < 0 || n != int(n) || !isFinite(frame.x) || !isFinite(frame.y)) return false;
                if (!checked[n]) {
                    var frameClass:String = prefix + "_" + int(n);
                    if (!domain.hasDefinition(frameClass)) { reason = "frame-missing"; prepareMovie(res,source,id,type,config); return false; }
                    if (!extendsType(domain, frameClass, BitmapData)) { reason = "class-mismatch"; return false; }
                    checked[n] = true;
                }
            }
            return remember(domain,key);
        }

        private static function remember(domain:ApplicationDomain,key:String):Boolean {
            var keys:Array = verified[domain] as Array;
            if (!keys) verified[domain] = keys = [];
            if (keys.length >= 32) keys.shift();
            keys.push(key);
            return true;
        }

        private static function prepareMovie(res:Object,source:int,id:int,type:int,config:Object):void {
            var count:int = config.MOVIE_ICON_SPLIT_FILE_NUM;
            if (count < 1 || count > 16) return;
            var urls:Array = [];
            for (var i:int = 0; i < count; i++) urls.push(config.getMovieIconRes(id,type,i,-1));
            prepare(res,source,id,urls);
        }

        private static function prepare(res:Object,source:int,id:int,urls:Array):void {
            var domain:ApplicationDomain = res.applicationDomain;
            if (active) { if (active.id == id) reason = "preload-pending"; return; }
            var recent:Object = cooldown[domain];
            if (recent && recent.id == id && uint(getTimer() - recent.time) < 30000) {
                if (recent.reason) reason = recent.reason;
                return;
            }
            var binding:Number = Number(ExternalInterface.call("aolaMyProBinding"));
            if (!isFinite(binding) || binding <= 0) return;
            var job:Object = {domain:domain,res:res,binding:binding,source:source,id:id,urls:urls,next:0,url:null,done:null,
                scheduler:getDefinitionByName("mmo.loader.scheduler.ResScheduler"),timer:0,deferred:0};
            active = job;
            // Start after the game has mounted its original view. A load completion only
            // advances this private queue; it never remounts or mutates a display object.
            job.deferred = setTimeout(function():void {advance(job);},1);
        }

        private static function advance(job:Object):void {
            if (active !== job) return;
            try {
                var sub:Object = getDefinitionByName("mmo.loader.scheduler.SubDomain");
                if (sub.getInstance().getPetViewDomain().applicationDomain !== job.domain) { finish(job); return; }
                if (Number(ExternalInterface.call("aolaMyProBinding")) !== job.binding ||
                    int(ExternalInterface.call("aolaMyProModelCandidate","backpack",job.source,false)) != job.id) { finish(job); return; }
                if (job.next >= job.urls.length) { progress(job,"loaded"); finish(job,null,true); return; }
                var params:Object = getDefinitionByName("mmo.loader.scheduler.ResParams");
                var priorities:Object = getDefinitionByName("mmo.loader.scheduler.ResPriority");
                var url:String = job.urls[job.next++];
                job.url = url;
                progress(job,"loading");
                // A four-part HD movie can outlast one request's timeout in total.
                job.timer = setTimeout(function():void { if (job.url === url) finish(job,"preload-timeout"); },20000);
                job.done = function(...ignored):void {
                    if (active !== job || job.url !== url) return;
                    clearTimeout(job.timer);
                    job.url = null; job.done = null;
                    job.deferred = setTimeout(function():void {advance(job);},1);
                };
                var failed:Function = function(...ignored):void { if (job.url === url) finish(job,"preload-error"); };
                job.scheduler.getRes(params.create(url,job.done,job.res,priorities.PRELOAD,null,failed));
            } catch(error:Error) { finish(job,"preload-error"); }
        }

        private static function progress(job:Object,stage:String):void {
            try { ExternalInterface.call("aolaMyProModelProgress","backpack",job.source,job.id,false,job.binding,stage,job.next,job.urls.length); }
            catch (unused:Error) {}
        }

        private static function finish(job:Object,failure:String = null,complete:Boolean = false):void {
            if (!job || active !== job) return;
            active = null;
            clearTimeout(job.timer);
            clearTimeout(job.deferred);
            if (failure) progress(job,failure == "preload-timeout" ? "timeout" : "load-error");
            if (failure || complete) cooldown[job.domain] = {id:job.id,time:getTimer(),reason:failure};
            if (job.url && job.done != null) {
                try { job.scheduler.cancel(job.url,job.done); } catch(error:Error) {}
            }
            job.url = null; job.done = null;
        }

        private static function extendsType(domain:ApplicationDomain, name:String, base:Class):Boolean {
            if (!domain.hasDefinition(name)) return false;
            var cls:Class = domain.getDefinition(name) as Class;
            return cls != null && (cls == base || base.prototype.isPrototypeOf(cls.prototype));
        }
    }
}
