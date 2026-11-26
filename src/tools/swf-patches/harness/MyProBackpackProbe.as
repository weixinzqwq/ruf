package {
    import flash.display.MovieClip;
    import flash.display.Sprite;
    import flash.geom.Point;
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

            var config:Object = getDefinitionByName("mmo.interfaces.pet.peticon.PetIconConfig");
            var nativeHDGroups:Boolean = true;
            var id:int;
            for each (id in [5994, 5995, 6000, 6044]) {
                nativeHDGroups = nativeHDGroups && close(config.getPetMovieHDScale(id), 0.3);
            }
            for each (id in [5980, 5997, 5998, 6007]) {
                nativeHDGroups = nativeHDGroups && close(config.getPetMovieHDScale(id), 0.4);
            }
            trace("native scale groups", nativeHDGroups && close(config.getPetMovieHDScale(5336), 1));

            checkScale(helper, "normal to .3", 5336, 6044, 2, 0.3);
            checkScale(helper, "fresh attachment", 5336, 6044, 2, 0.3);
            checkScale(helper, ".3 to normal", 6044, 5336, 2, 1);
            checkScale(helper, ".3 to .4", 6044, 5997, 2, 0.4);
            checkScale(helper, ".4 to .3", 5980, 5994, 2, 0.3);
            checkScale(helper, "same scale group", 6044, 6000, 2, 0.3);
            checkScale(helper, "no mapping", 6044, 0, 2, 0.3);
            checkScale(helper, "bitmap ignores mapping", 5336, 6044, 4, 1);
            checkScale(helper, "unexpected hierarchy", 5336, 6044, 2, 1, false);
            ModelHost.available = false;
            checkScale(helper, "missing host keeps scale", 6044, 5336, 2, 0.3);
        }

        private function checkScale(helper:Object, label:String, source:int, target:int, type:int,
                                    expected:Number, expectedHierarchy:Boolean = true):void {
            var config:Object = getDefinitionByName("mmo.interfaces.pet.peticon.PetIconConfig");
            var outer:Sprite = new Sprite();
            outer.x = 112;
            outer.y = -48;
            outer.scaleX = 1.25;
            outer.scaleY = 0.8;
            var icon:Sprite = new Sprite();
            icon.x = 9;
            icon.y = 23;
            outer.addChild(icon);
            var model:MovieClip = new MovieClip();
            model.x = 17;
            model.y = -12;
            model.scaleX = model.scaleY = config.getPetMovieHDScale(source);
            icon.addChild(model);
            var art:Sprite = new Sprite();
            art.x = 3;
            art.y = 5;
            art.graphics.beginFill(0);
            art.graphics.drawRect(-80, -180, 160, 180);
            art.graphics.endFill();
            model.addChild(art);

            var originalScale:Number = model.scaleX;
            var origin:Point = model.localToGlobal(new Point());
            var width:Number = model.width;
            var height:Number = model.height;
            ModelHost.backpackTarget = target;
            ModelHost.queries = 0;
            ModelHost.lastSource = 0;
            var handler:Object = {iconRes: art, container: expectedHierarchy ? outer : icon};
            var loaded:Function = helper.scaleOnLoaded(handler, source, type);
            if (loaded != null) {
                loaded();
            }
            var scaleCorrect:Boolean = close(model.scaleX, expected) && close(model.scaleY, expected)
                && close(model.width, width * expected / originalScale)
                && close(model.height, height * expected / originalScale);
            var firstX:Number = model.scaleX;
            var firstY:Number = model.scaleY;
            if (loaded != null) {
                loaded();
            }
            var newOrigin:Point = model.localToGlobal(new Point());
            var positionCorrect:Boolean = model.x == 17 && model.y == -12 && art.x == 3 && art.y == 5
                && icon.x == 9 && icon.y == 23 && outer.x == 112 && outer.y == -48
                && close(newOrigin.x, origin.x) && close(newOrigin.y, origin.y);
            var sourceCorrect:Boolean = ModelHost.queries <= 1
                && (ModelHost.queries == 0 ? close(originalScale, expected) : ModelHost.lastSource == source);
            trace(label, scaleCorrect, close(model.scaleX, firstX) && close(model.scaleY, firstY),
                  positionCorrect, sourceCorrect);
        }

        private function close(value:Number, expected:Number):Boolean {
            return Math.abs(value - expected) < 0.00001;
        }
    }
}

package probe {
    public class ModelHost {
        public static var available:Boolean = true;
        public static var revision:* = 0;
        public static var backpackTarget:int = 0;
        public static var queries:int = 0;
        public static var lastSource:int = 0;

        public static function call(name:String, source:int = 0):* {
            if (name == "aolaMyProMorphRevision") return revision;
            if (name == "aolaMyProBackpackTarget") {
                queries++;
                lastSource = source;
                return backpackTarget;
            }
            throw new Error(name);
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
