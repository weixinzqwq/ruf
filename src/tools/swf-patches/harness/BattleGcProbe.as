package {
    import flash.display.Sprite;
    import flash.events.Event;
    import flash.utils.getDefinitionByName;
    import probe.BattleTail;

    public class BattleGcProbe extends Sprite {
        private var step:int;
        private var panel:Object;
        private var panelClass:Class;

        public function BattleGcProbe() {
            panelClass = getDefinitionByName("mmo.petfightnew.common.FightPanel") as Class;
            addEventListener(Event.ENTER_FRAME, advance);
        }

        private function advance(e:Event):void {
            switch (++step) {
                case 1:
                    panel = new panelClass();
                    panel.disposeManagers(); // a direct call cannot confirm exit
                    trace("standalone managers");
                    break;
                case 2:
                    panel.dispose();
                    panel.dispose(); // original _isDisposed early return
                    trace("normal then early", BattleTail.calls);
                    break;
                case 3:
                    panel = new panelClass();
                    BattleTail.fail = true;
                    try { panel.dispose(); }
                    catch (error:Error) { trace("tail threw", error.message); }
                    panel.dispose(); // unsuccessful tail must not leave a stale marker
                    break;
                case 4:
                    panel = new panelClass();
                    BattleTail.replace = function():void { panel = new panelClass(); };
                    panel.dispose(); // next constructor returns inside the old dispose call
                    trace("replacement entered", BattleTail.calls);
                    break;
                case 5:
                    panel.dispose();
                    trace("replacement exited", BattleTail.calls);
                    removeEventListener(Event.ENTER_FRAME, advance);
                    break;
            }
        }
    }
}
package probe {
    public class BattleTail {
        public static var fail:Boolean;
        public static var calls:int;
        public static var replace:Function;
        public static function afterManagers():void {
            ++calls;
            if (fail) {
                fail = false;
                throw new Error("after managers");
            }
            if (replace != null) {
                var callback:Function = replace;
                replace = null;
                callback();
            }
        }
    }
}
