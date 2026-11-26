package flash.utils {
    [Ruffle(InstanceAllocator)]
    public dynamic class Dictionary {
        private native function enableWeakKeys():void;
        public native function aoqiUseDisplayOwners():Dictionary;
        prototype.toJSON = function(r:String):* {
            return "Dictionary";
        };
        prototype.setPropertyIsEnumerable("toJSON", false);

        public function Dictionary(weakKeys:Boolean = false) {
            if (weakKeys) {
                enableWeakKeys();
            }
        }
    }
}
