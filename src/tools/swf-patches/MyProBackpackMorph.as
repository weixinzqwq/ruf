package aola.tools {
    import flash.external.ExternalInterface;
    import flash.utils.getDefinitionByName;

    public final class MyProBackpackMorph {
        private static var lastRevision:Number = 0;

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
