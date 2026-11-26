package aola.tools {
    import flash.display.DisplayObject;
    import flash.external.ExternalInterface;
    import flash.utils.getDefinitionByName;

    public final class MyProBackpackMorph {
        private static var lastRevision:Number = 0;

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
