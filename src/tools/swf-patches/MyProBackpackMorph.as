package aola.tools {
    import flash.display.DisplayObject;
    import flash.events.Event;
    import flash.events.IOErrorEvent;
    import flash.external.ExternalInterface;
    import flash.media.Sound;
    import flash.media.SoundChannel;
    import flash.net.URLRequest;
    import flash.utils.Dictionary;
    import flash.utils.getDefinitionByName;

    public final class MyProBackpackMorph {
        private static var lastRevision:Number = 0;
        private static var view:DisplayObject;
        private static var updateView:Function;
        private static var updateIcon:Function;
        private static var resetIcon:Function;
        private static var updateControls:Function;
        private static var updateHeads:Dictionary = new Dictionary();
        private static var refreshingView:Boolean;
        private static var activeBackground:DisplayObject;
        private static var backgroundGeneration:uint;
        private static var sound:Sound;
        private static var channel:SoundChannel;

        public static function visualSkinId(service:Object, viewId:uint, sourceSkinId:uint):uint {
            if (!ExternalInterface.available || !service || !viewId) return sourceSkinId;
            try {
                var candidate:uint = uint(ExternalInterface.call("aolaMyProVisualSkinId", viewId, sourceSkinId));
                if (candidate) return service.getPetSkinInfoBySkinId(candidate) ? candidate : sourceSkinId;
                candidate = uint(ExternalInterface.call("aolaMyProBackpackTarget", viewId));
                if (candidate && service.getPetSkinInfoBySkinId(candidate)) {
                    var confirmed:uint = uint(ExternalInterface.call(
                        "aolaMyProVisualSkinId", viewId, sourceSkinId, candidate));
                    if (confirmed == candidate) return candidate;
                }
            } catch (error:Error) {
            }
            return sourceSkinId;
        }

        public static function sourceSkinId(pet:Object):uint {
            return pet.specialSkinId > 0 ? pet.specialSkinId : pet.viewId;
        }

        public static function packageOpen(original:Boolean, service:Object, pet:Object,
                                           openWhenMapped:Boolean):Boolean {
            var skinId:uint = sourceSkinId(pet);
            return visualSkinId(service, pet.viewId, skinId) == skinId ? original : openWhenMapped;
        }

        public static function preMoviePreference(raw:uint, service:Object, pet:Object):uint {
            return visualSkinId(service, pet.viewId, raw) == raw ? raw : pet.viewId;
        }

        public static function preMovieSkinId(original:uint, service:Object, pet:Object):uint {
            var source:uint = sourceSkinId(pet);
            var mapped:uint = visualSkinId(service, pet.viewId, source);
            return mapped == source ? original : mapped;
        }

        public static function useDIYBackground(original:int, service:Object, pet:Object):int {
            var skinId:uint = sourceSkinId(pet);
            return visualSkinId(service, pet.viewId, skinId) == skinId ? original : 0;
        }

        public static function showPetModel(original:Boolean, service:Object, pet:Object):Boolean {
            var id:uint = sourceSkinId(pet);
            var mapped:uint = visualSkinId(service, pet.viewId, id);
            return id == mapped ? original : service.getPetSkinInfoBySkinId(mapped).showPet;
        }

        public static function hideModelAfterMappedPreMovie(service:Object, pet:Object):Boolean {
            return pet && !showPetModel(true, service, pet);
        }

        public static function unchangedSelection(unchanged:Boolean):Boolean {
            return unchanged && !refreshingView;
        }

        public static function beginBackground(bg:DisplayObject):void {
            activeBackground = bg;
            backgroundGeneration++;
        }

        public static function forgetBackground(bg:DisplayObject):void {
            if (activeBackground === bg) beginBackground(null);
        }

        public static function guardBackground(bg:DisplayObject, callback:Function):Function {
            var generation:uint = backgroundGeneration;
            return function(...args):void {
                if (bg && bg.parent && bg === activeBackground && backgroundGeneration == generation) {
                    callback.apply(null, args);
                }
            };
        }

        public static function registerView(owner:DisplayObject, callback:Function):void {
            if (!ExternalInterface.available) return;
            view = owner;
            updateView = callback;
            try {
                ExternalInterface.addCallback("onAolaMyProVisualRefresh", refreshView);
            } catch (error:Error) {
            }
        }

        public static function unregisterView(owner:DisplayObject):void {
            if (view !== owner) return;
            view = null;
            updateView = updateIcon = resetIcon = updateControls = null;
            updateHeads = new Dictionary();
            stopTalk();
            try {
                ExternalInterface.addCallback("onAolaMyProVisualRefresh", null);
            } catch (error:Error) {
            }
        }

        public static function registerIcon(reset:Function, update:Function):void {
            if (!view) return;
            resetIcon = reset;
            updateIcon = update;
        }

        public static function registerControls(preMovie:Function, talk:Function, caiDan:Function):void {
            if (!view) return;
            updateControls = function():void {
                preMovie();
                talk();
                caiDan();
            };
        }

        public static function registerHead(owner:DisplayObject, update:Function, loader:Object):void {
            if (!view || !loader) return;
            updateHeads[owner] = function():void {
                loader.dispose();
                update();
            };
        }

        private static function refreshView():Boolean {
            if (!view || !view.stage || !updateIcon) return false;
            refresh();
            stopTalk();
            refreshingView = true;
            try {
                if (resetIcon) resetIcon();
                updateIcon(null);
                if (updateControls) updateControls();
                for (var owner:Object in updateHeads) {
                    if (owner.stage && owner.visible) updateHeads[owner]();
                }
                updateView();
            } finally {
                refreshingView = false;
            }
            return true;
        }

        public static function stopTalkOnSelection(current:Object, next:Object):void {
            if (!current || !next || current.id != next.id || current.viewId != next.viewId ||
                sourceSkinId(current) != sourceSkinId(next)) stopTalk();
        }

        public static function handleTalk(service:Object, pet:Object, original:uint):Boolean {
            var skinId:uint = visualSkinId(service, pet.viewId, original);
            var info:Object = service.getPetSkinInfoBySkinId(skinId);
            if (info && info.unlockTalkItemId) return skinId != original;
            playTalk(service, skinId);
            return true;
        }

        public static function showTalkButton(original:Boolean, service:Object, pet:Object):Boolean {
            if (!original) return false;
            var id:uint = sourceSkinId(pet);
            var mapped:uint = visualSkinId(service, pet.viewId, id);
            return mapped == id || !service.getPetSkinInfoBySkinId(mapped).unlockTalkItemId;
        }

        public static function playTalk(service:Object, skinId:uint):void {
            stopTalk();
            var info:Object = service.getPetSkinInfoBySkinId(skinId);
            if (!info || !info.hasTalk) return;
            try {
                var versions:Object = getDefinitionByName("mmo.loader.scheduler.VersManager");
                sound = new Sound();
                sound.addEventListener(Event.COMPLETE, onTalkLoaded);
                sound.addEventListener(IOErrorEvent.IO_ERROR, stopTalk);
                sound.load(versions.getVerURLRequest(info.talkRes) as URLRequest);
            } catch (error:Error) {
                stopTalk();
            }
        }

        private static function onTalkLoaded(event:Event):void {
            if (event.target !== sound) return;
            channel = sound.play();
            if (channel) channel.addEventListener(Event.SOUND_COMPLETE, stopTalk);
            else stopTalk();
        }

        public static function stopTalk(event:Event = null):void {
            if (sound) {
                sound.removeEventListener(Event.COMPLETE, onTalkLoaded);
                sound.removeEventListener(IOErrorEvent.IO_ERROR, stopTalk);
                try { sound.close(); } catch (error:Error) {}
                sound = null;
            }
            if (channel) {
                channel.removeEventListener(Event.SOUND_COMPLETE, stopTalk);
                channel.stop();
                channel = null;
            }
        }

        public static function scaleOnLoaded(handler:Object, sourceId:int, iconType:int):Function {
            if (!ExternalInterface.available) return null;
            var config:Object;
            try {
                config = getDefinitionByName("mmo.interfaces.pet.peticon.PetIconConfig");
                if (!config.isNewMovieIcon(sourceId, iconType)) return null;
            } catch (error:Error) {
                return null;
            }
            var handled:Boolean = false;
            return function():void {
                if (handled) return;
                handled = true;
                try {
                    var targetId:int = int(ExternalInterface.call("aolaMyProBackpackTarget", sourceId));
                    if (targetId <= 0 || targetId == sourceId) return;
                    var ratio:Number = config.getPetMovieHDScale(targetId) / config.getPetMovieHDScale(sourceId);
                    if (!isFinite(ratio) || ratio <= 0 || ratio == 1) return;

                    // iconRes is the inner bitmap movie. The HD scale belongs to
                    // its wrapper, within the icon already aligned by the game.
                    var icon:DisplayObject = handler.iconRes as DisplayObject;
                    var model:DisplayObject = icon ? icon.parent : null;
                    if (!model || !model.parent || model.parent.parent !== handler.container) return;
                    model.scaleX *= ratio;
                    model.scaleY *= ratio;
                } catch (error:Error) {
                }
            };
        }

        public static function refresh():Boolean {
            if (!ExternalInterface.available) return false;
            try {
                var reported:* = ExternalInterface.call("aolaMyProMorphRevision");
                if (reported === null || reported === undefined) return false;
                var revision:Number = Number(reported);
                if (!isFinite(revision) || revision < 0 || revision == lastRevision) return false;

                var subDomain:Object = getDefinitionByName("mmo.loader.scheduler.SubDomain");
                subDomain.getInstance().disposeDomain(subDomain.PET_VIEW_DOMAIN);
                lastRevision = revision;
                return true;
            } catch (error:Error) {
                return false;
            }
        }
    }
}
