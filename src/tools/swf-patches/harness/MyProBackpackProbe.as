package {
    import flash.display.Sprite;
    import flash.utils.getDefinitionByName;
    import mmo.loader.scheduler.SubDomain;
    import probe.ModelHost;

    public class MyProBackpackProbe extends Sprite {
        public function MyProBackpackProbe() {
            var helper:Object = getDefinitionByName("aola.tools.MyProBackpackMorph");
            var sub:SubDomain = SubDomain.getInstance();
            var domain:Object = sub.getPetViewDomain();
            trace("initial", helper.refresh(), domain === sub.getPetViewDomain());

            ModelHost.revision = 1;
            trace("rule changed", helper.refresh(), domain !== sub.getPetViewDomain());
            domain = sub.getPetViewDomain();
            trace("same rule", helper.refresh(), domain === sub.getPetViewDomain());

            ModelHost.revision = 2;
            trace("restore", helper.refresh(), domain !== sub.getPetViewDomain());
            domain = sub.getPetViewDomain();
            ModelHost.revision = null;
            trace("missing result", helper.refresh(), domain === sub.getPetViewDomain());
            ModelHost.revision = 3;
            ModelHost.available = false;
            trace("missing host", helper.refresh(), domain === sub.getPetViewDomain());
            ModelHost.available = true;
            trace("host resumed", helper.refresh(), domain !== sub.getPetViewDomain());
        }
    }
}

package probe {
    public class ModelHost {
        public static var available:Boolean = true;
        public static var revision:* = 0;

        public static function call(name:String):* {
            if (name != "aolaMyProMorphRevision") throw new Error(name);
            return revision;
        }
    }
}

package mmo.loader.scheduler {
    public class SubDomain {
        public static const PET_VIEW_DOMAIN:String = "0";
        private static var instance:SubDomain = new SubDomain();
        private var domain:Object;

        public static function getInstance():SubDomain {
            return instance;
        }

        public function getPetViewDomain():Object {
            if (domain == null) domain = {};
            return domain;
        }

        public function disposeDomain(id:String):void {
            if (id != PET_VIEW_DOMAIN) throw new Error(id);
            domain = null;
        }
    }
}
