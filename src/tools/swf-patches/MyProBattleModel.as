package aola.tools {
    import flash.display.BitmapData;
    import flash.display.MovieClip;
    import flash.display.Sprite;
    import flash.external.ExternalInterface;
    import flash.system.ApplicationDomain;
    import flash.utils.ByteArray;
    import flash.utils.Dictionary;
    import flash.utils.clearTimeout;
    import flash.utils.getDefinitionByName;
    import flash.utils.getTimer;
    import flash.utils.setTimeout;

    public final class MyProBattleModel {
        private static var verified:Dictionary = new Dictionary(true);
        private static var pending:Object = {};
        private static var recent:Dictionary = new Dictionary(true);
        // Called only at the ordinary display-key assignment. Combat identities stay on the original object.
        public static function body(source:int, face:Boolean):int {
            return choose("body", source, face);
        }

        public static function icon(source:int):int {
            return choose("icon", source, false);
        }

        private static function choose(kind:String, source:int, face:Boolean):int {
            if (source <= 0 || !ExternalInterface.available) return source;
            try {
                var target:int = int(ExternalInterface.call("aolaMyProModelCandidate", kind, source, face));
                if (target <= 0 || target == source) return source;
                var reason:String;
                try {
                    if (kind == "icon") {
                        reason = checkIcon(source, target);
                    } else {
                        reason = checkBody(source, target, face);
                    }
                } catch (missing:*) { reason = "display-check-error"; }
                var selected:int = int(ExternalInterface.call("aolaMyProModelUse", kind, source, target, face, reason == null, reason));
                if (!reason && selected == target) return selected;
            } catch (missingHost:*) {}
            return source;
        }

        private static function checkIcon(source:int, target:int):String {
            // Both FIGHT loaders choose the target's _F class at the normal scale.
            // PetIconContainer folds IDs modulo 10000 and substitutes a default for absent PM data.
            if (target >= 10000) return "unsupported-id";
            var pool:Object = getDefinitionByName("mmo.petfightnew.common.manager.ServicePool");
            var pm:Object = pool.instance.getService(getDefinitionByName("mmo.interfaces.pet.pm.services.IPMService"));
            if (!pm.customData.pmDataMap.getPM(target)) return "pm-data-missing";
            var domain:ApplicationDomain = viewDomain();
            var name:String = "mmo.pet.peticon" + target + "_F";
            if (domain.hasDefinition(name)) return hasClass(domain, name, Sprite) ? null : "class-mismatch";
            var path:String = target >= 4400 ? "peticon/petfighticon/peticon" + target : "peticon/peticon" + target;
            prepare("icon", source, target, false, domain, [path]);
            return "config-missing";
        }

        private static function viewDomain():ApplicationDomain {
            return getDefinitionByName("mmo.petfightnew.common.FightData").instance.getPetViewDomain().applicationDomain;
        }

        private static function hasClass(domain:ApplicationDomain, name:String, type:Class):Boolean {
            if (!domain.hasDefinition(name)) return false;
            var cls:Class = domain.getDefinition(name) as Class;
            return cls && (cls == type || type.prototype.isPrototypeOf(cls.prototype));
        }

        private static function checkBody(source:int, target:int, face:Boolean):String {
            var data:Object = getDefinitionByName("mmo.petfightnew.common.FightData").instance;
            var config:Object = getDefinitionByName("mmo.play.petfight.common.config.FightResConfig");
            var strategies:Object = getDefinitionByName("mmo.play.petfight.common.PetMovieStrategyType");
            var domain:ApplicationDomain = data.getPetViewDomain().applicationDomain;
            var side:int = face ? 1 : 2; // PetFace.POSITIVE / BACK in this game profile
            var strategy:int = data.getPetMovieStrategyType(target);
            var vector:Boolean = strategy == strategies.VECTORGRAPH;
            var key:String = target + "/" + side + "/" + strategy;
            var keys:Array = verified[domain] as Array;
            if (keys && keys.indexOf(key) >= 0) return null;
            var name:String = vector ? config.getSinglePetMovieClsName(target, side) : config.getPetBitmapMovieConfigClsName(target, side);
            var missingMain:Boolean = !domain.hasDefinition(name);
            var reason:String = missingMain ? "config-missing" : null;
            if (!missingMain) {
                if (vector) return hasClass(domain, name, MovieClip) ? null : "class-mismatch";
                reason = checkBitmap(domain, name, target, side);
            }
            if (reason) {
                var paths:Array = [];
                if (missingMain) paths.push(config.getPetMainFilePath(strategy,target,side));
                // The ordinary preloader requests the original pet's remaining files only.
                // Prepare the mapped pet's complementary files in the domain used for display.
                if ((missingMain || reason == "frame-missing") && !vector && config.isComplementaryPetMovie(target,side)) {
                    for (var part:int=1; part<int(config.FILE_NUM_OF_COMPLEMENTARY_PET_MOVIE); ++part) {
                        paths.push(config.getFilePathOfComplementaryPetMovie(target,side,part));
                    }
                }
                if (paths.length) prepare("body", source, target, face, domain, paths);
                return reason;
            }
            if (!keys) verified[domain] = keys = [];
            if (keys.length >= 32) keys.shift();
            keys.push(key);
            return null;
        }

        private static function checkBitmap(domain:ApplicationDomain, name:String, target:int, side:int):String {
            var ConfigMovie:Class = domain.getDefinition(name) as Class;
            var movie:MovieClip = new ConfigMovie() as MovieClip;
            if (movie == null || !(movie["MovieConfig"] is Class)) return "invalid-config";
            var ConfigBytes:Class = movie["MovieConfig"];
            var dataBytes:ByteArray = new ConfigBytes() as ByteArray;
            if (dataBytes == null || dataBytes.length == 0 || dataBytes.length > 65536) return "invalid-config";
            dataBytes.position = 0;
            var entries:Array = dataBytes.readUTFBytes(dataBytes.length).split("#");
            var standing:Boolean = false;
            var seen:Object = {};
            for each (var entry:String in entries) {
                var separator:int = entry.indexOf(":");
                if (separator < 0) continue;
                var key:Array = entry.substr(0, separator).split("$");
                var frame:String = key.length > 1 ? key[1] : key[0];
                if (!/^[0-9]+$/.test(frame)) return "invalid-config";
                var frames:Array = entry.substr(separator + 1).split(";");
                for each (var fragment:String in frames) {
                    var image:String = fragment.split(",")[0];
                    if (!/^[0-9]+$/.test(image)) return "invalid-config";
                    var imageClass:String = "mmo.petfight.bitmap.pet" + target + "_" + side + "_" + frame + "_" + int(image);
                    if (seen[imageClass]) continue;
                    if (!domain.hasDefinition(imageClass)) return "frame-missing";
                    if (!hasClass(domain, imageClass, BitmapData)) return "class-mismatch";
                    seen[imageClass] = true;
                }
                if (frame == "1") standing = true;
            }
            return standing ? null : "invalid-config";
        }

        // There is at most one request per display kind. Never retain a fight container
        // or replace a live object from a callback. A later ordinary selector checks the domain.
        private static function prepare(kind:String, source:int, target:int, face:Boolean, domain:ApplicationDomain, paths:Array):void {
            var binding:Number = Number(ExternalInterface.call("aolaMyProBinding"));
            if (!(binding > 0)) return;
            var previous:Object = pending[kind];
            if (previous) {
                if (previous.domain === domain && previous.binding == binding) return;
                finish(previous, true);
            }
            var stamp:String = kind + "/" + target + "/" + int(face) + "/" + binding;
            var last:Object = recent[domain];
            if (last && last[kind] && last[kind].key == stamp && uint(getTimer() - last[kind].time) < 30000) return;
            var job:Object = {kind:kind, source:source, target:target, face:face, binding:binding,
                domain:domain, key:stamp, paths:paths, index:0, path:null, timer:0, deferred:0, done:null, scheduler:null};
            pending[kind] = job;
            job.deferred = setTimeout(loadNext, 1, job);
        }

        private static function progress(job:Object, stage:String):void {
            try {
                ExternalInterface.call("aolaMyProModelProgress", job.kind, job.source, job.target,
                    job.face, job.binding, stage, job.index + 1, job.paths.length);
            } catch (unusedHost:*) {}
        }

        private static function loadNext(job:Object):void {
            try {
                if (pending[job.kind] !== job || viewDomain() !== job.domain ||
                    Number(ExternalInterface.call("aolaMyProBinding")) != job.binding ||
                    int(ExternalInterface.call("aolaMyProModelCandidate", job.kind, job.source, job.face)) != job.target) {
                    finish(job, true); return;
                }
                var path:String = job.path = job.paths[job.index];
                var priority:Object = getDefinitionByName("mmo.loader.scheduler.ResPriority");
                job.scheduler = getDefinitionByName("mmo.loader.scheduler.ResScheduler");
                job.done = function(...args):void {
                    if (pending[job.kind] !== job || path !== job.path) return;
                    clearTimeout(job.timer);
                    job.path = null;
                    if (job.index + 1 >= job.paths.length) {
                        progress(job, "loaded");
                        finish(job, false);
                    } else {
                        ++job.index;
                        job.deferred = setTimeout(loadNext, 1, job);
                    }
                };
                var failed:Function = function(...args):void {
                    if (pending[job.kind] !== job || path !== job.path) return;
                    progress(job, "load-error"); finish(job, false);
                };
                job.timer = setTimeout(function():void {
                    progress(job, "timeout"); finish(job, true);
                }, 15000);
                progress(job, "loading");
                job.scheduler.getResource(path, job.done, "", failed, null, priority.PRELOAD, false, job.domain);
            } catch (missing:*) { progress(job, "load-error"); finish(job, true); }
        }

        private static function finish(job:Object, cancel:Boolean):void {
            if (pending[job.kind] !== job) return;
            delete pending[job.kind];
            clearTimeout(job.timer); clearTimeout(job.deferred);
            if (cancel && job.path && job.scheduler && job.done != null) {
                try { job.scheduler.cancel(job.path, job.done); } catch (gone:*) {}
            }
            var last:Object = recent[job.domain];
            if (!last) recent[job.domain] = last = {};
            last[job.kind] = {key:job.key, time:getTimer()};
            job.done = null;
            job.domain = null;
        }
    }
}
