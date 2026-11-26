package aola.tools {
    import flash.external.ExternalInterface;

    public final class MyProBattleEffects {
        private static function enabled(name:String):Boolean {
            try {
                return ExternalInterface.available && ExternalInterface.call(name) === true;
            } catch (error:Error) {
                return false;
            }
        }

        public static function damage():Boolean { return enabled("aolaHideBattleDamage"); }
        public static function healing():Boolean { return enabled("aolaHideBattleHealing"); }
        public static function skip():Boolean { return enabled("aolaSkipBattleAnimations"); }

        public static function multiple(delta:Number):Boolean {
            if (delta < 0) return damage();
            if (delta > 0) return healing();
            return false;
        }
    }
}
