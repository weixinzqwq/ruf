package mmo.petpackage.data {
    public class PetDetailDataMap {
        public function dispatch(fail:Boolean):void {
            tryDispatcherCurrentDetailedCachedEvent(fail);
        }

        private function tryDispatcherCurrentDetailedCachedEvent(fail:Boolean):void {
            if (fail) throw new Error("fixture");
        }
    }
}

package mmo.pet.dataservice {
    import mmo.petpackage.data.PetDetailDataMap;

    public class PetDataService {
        public function onViewSinglePet(cache:PetDetailDataMap):void {
            cache.dispatch(true);
        }
    }
}

package {
    import flash.display.MovieClip;
    import flash.events.TimerEvent;
    import flash.utils.Timer;
    import mmo.pet.dataservice.PetDataService;
    import mmo.petpackage.data.PetDetailDataMap;

    public class Test extends MovieClip {
        private var timer:Timer = new Timer(40, 1);
        private var cache:PetDetailDataMap = new PetDetailDataMap();

        public function Test() {
            timer.addEventListener(TimerEvent.TIMER, run);
            timer.start();
        }

        private function run(event:TimerEvent):void {
            try {
                new PetDataService().onViewSinglePet(cache);
            } catch (error:Error) {
                trace(error.message);
            }
            cache.dispatch(false);
            trace("complete");
        }
    }
}
