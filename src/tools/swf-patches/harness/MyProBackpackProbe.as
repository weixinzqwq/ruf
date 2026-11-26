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

            checkVisualSkin(helper);
            checkVisualRefresh(helper, sub);
            checkBackgroundLifetime(helper);
            checkVoiceRouting(helper);

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

        private function checkVisualSkin(helper:Object):void {
            var sourceViewId:uint = 5336;
            var sourceSkinId:uint = 533601;
            var service:Object = {
                getPetSkinInfoBySkinId: function(id:uint):Object {
                    return id == 5997 || id == 6044 ? {skinId: id} : null;
                }
            };
            ModelHost.backpackTarget = 6044;
            ModelHost.posterTarget = 5997;
            ModelHost.visualQueries = [];
            trace("visual poster priority", helper.visualSkinId(service, sourceViewId, sourceSkinId),
                  ModelHost.visualQueries.join(";"));

            ModelHost.posterTarget = 6000;
            trace("visual invalid explicit poster", helper.visualSkinId(service, sourceViewId, sourceSkinId));

            ModelHost.posterTarget = 0;
            ModelHost.visualQueries = [];
            trace("visual backpack skin", helper.visualSkinId(service, sourceViewId, sourceSkinId),
                  ModelHost.visualQueries.join(";"));

            ModelHost.backpackTarget = 6000;
            ModelHost.visualQueries = [];
            trace("visual missing skin record", helper.visualSkinId(service, sourceViewId, sourceSkinId),
                  ModelHost.visualQueries.join(";"));

            ModelHost.backpackTarget = 6044;
            ModelHost.confirmedTarget = 6000;
            trace("visual confirmation changed", helper.visualSkinId(service, sourceViewId, sourceSkinId));
            ModelHost.confirmedTarget = 0;
            ModelHost.backpackTarget = 0;
            trace("visual rule restored", helper.visualSkinId(service, sourceViewId, sourceSkinId));
            ModelHost.available = false;
            ModelHost.visualQueries = [];
            trace("visual absent host", helper.visualSkinId(service, sourceViewId, sourceSkinId),
                  ModelHost.visualQueries.length);
            ModelHost.available = true;
        }

        private function checkVisualRefresh(helper:Object, sub:SubDomain):void {
            var owner:Sprite = new Sprite();
            var head:Sprite = new Sprite();
            var offstage:Sprite = new Sprite();
            owner.addChild(head);
            addChild(owner);
            var refreshed:int = 0;
            var heads:int = 0;
            var headDisposals:int = 0;
            var controls:int = 0;
            var iconResets:int = 0;
            var iconUpdates:int = 0;
            var lateHeads:int = 0;
            var updateControl:Function = function():void { controls++; };
            helper.registerView(owner, function():void { refreshed++; });
            helper.registerControls(updateControl, updateControl, updateControl);
            helper.registerIcon(function():void { iconResets++; }, function(event:Object):void {
                if (event == null) iconUpdates++;
            });
            helper.registerHead(head, function():void { heads++; }, {
                dispose: function():void { headDisposals++; }
            });
            helper.registerHead(offstage, function():void { lateHeads++; }, {
                dispose: function():void { lateHeads++; }
            });
            var callback:Function = ModelHost.callbacks.onAolaMyProVisualRefresh;
            trace("visual refresh callback registered", callback != null);
            if (callback == null) return;

            var domain:Object = sub.getPetViewDomain();
            ModelHost.revision = 4;
            callback();
            trace("visual refresh updates active view", refreshed, domain !== sub.getPetViewDomain());
            trace("visual refresh updates controls and icons", controls, iconResets, iconUpdates, heads, headDisposals,
                  lateHeads);

            helper.unregisterView(owner);
            ModelHost.revision = 5;
            callback();
            trace("visual refresh drops closed view", refreshed, controls, iconResets, iconUpdates, heads,
                  headDisposals, lateHeads);
            removeChild(owner);
        }

        private function checkBackgroundLifetime(helper:Object):void {
            var background:Sprite = new Sprite();
            addChild(background);
            var generations:Array = [];
            var capture:Function = function(value:String):void { generations.push(value); };
            helper.beginBackground(background);
            var beforeSwitch:Function = helper.guardBackground(background, capture);
            beforeSwitch("current");
            helper.beginBackground(background);
            var beforeClose:Function = helper.guardBackground(background, capture);
            beforeSwitch("old selection");
            beforeClose("replacement");
            helper.forgetBackground(background);
            beforeClose("closing");
            removeChild(background);
            beforeClose("closed");
            trace("visual background guards stale loads", generations.join(","));
        }

        private function checkVoiceRouting(helper:Object):void {
            var pet:Object = {viewId: 5336, specialSkinId: 533601};
            var queries:Array = [];
            var service:Object = {
                getPetSkinInfoBySkinId: function(id:uint):Object {
                    queries.push(id);
                    return {unlockTalkItemId: id == 5997 || id == 533601 ? 1 : 0, hasTalk: false};
                }
            };
            ModelHost.posterTarget = 6044;
            trace("visual normal voice handled locally", helper.handleTalk(service, pet, 533601), queries.join(","));
            ModelHost.posterTarget = 5997;
            queries.length = 0;
            trace("visual mapped unlock stays local", helper.handleTalk(service, pet, 533601), queries.join(","));
            ModelHost.posterTarget = 0;
            queries.length = 0;
            trace("visual official unlock retained", helper.handleTalk(service, pet, 533601), queries.join(","));
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
        public static var posterTarget:int = 0;
        public static var confirmedTarget:int = 0;
        public static var visualQueries:Array = [];
        public static var callbacks:Object = {};
        public static var queries:int = 0;
        public static var lastSource:int = 0;

        public static function addCallback(name:String, callback:Function):void {
            callbacks[name] = callback;
        }

        public static function call(name:String, source:int = 0, skin:int = 0, confirmed:int = 0):* {
            if (name == "aolaMyProMorphRevision") return revision;
            if (name == "aolaMyProBackpackTarget") {
                queries++;
                lastSource = source;
                return backpackTarget;
            }
            if (name == "aolaMyProVisualSkinId") {
                visualQueries.push([source, skin, confirmed].join(","));
                return posterTarget || (confirmed && (confirmedTarget || confirmed));
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
