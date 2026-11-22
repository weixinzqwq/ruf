package {
    import flash.display.Sprite;
    import flash.events.Event;
    import flash.geom.Point;
    import flash.system.fscommand;
    import flash.utils.getDefinitionByName;

    // A fresh sprite substitutes for panel artwork. The manager, style, motion,
    // and tween engine all come from the original SWFs' DoABC tags.
    public class SkinMotionProbe extends Sprite {
        private var managerType:Class;
        private var tweens:Class;
        private var manager:Object;
        private var target:Sprite;
        private var owned:Object;
        private var foreign:Object;
        private var retired:Array = [];
        private var frame:int = 0;
        private var cycle:int = 0;
        private var measured:int = 0;
        private var activeAfterClose:int = 0;
        private var movedAfterClose:int = 0;
        private var foreignActive:int = 0;
        private var foreignCompletions:int = 0;
        private var callbacksByTarget:Array = [];
        private var samples:Array;
        private var closedAt:Array;

        public function SkinMotionProbe() {
            managerType = getDefinitionByName("mmo.petpackage.controller.skin.PackageSkinManager") as Class;
            tweens = getDefinitionByName("com.greensock.TweenLite") as Class;
            manager = managerType["instance"];
            samples = addFiveStyles(manager);
            manager.changeClassify(1);
            addEventListener(Event.ENTER_FRAME, tick, false, -10);
        }

        private function tick(event:Event):void {
            ++frame;
            if (frame == 40) {
                trace("naturalOut=" + positions(samples).join("|"));
                manager.changeClassify(0);
            }
            if (frame == 80) {
                trace("naturalBack=" + positions(samples).join("|"));
                manager.dispose();
                for each (var sprite:Sprite in samples) removeChild(sprite);
                openNext();
            }
            if (frame > 80 && cycle < 35) {
                var phase:int = (frame - 80) % 4;
                if (phase == 2) {
                    manager.dispose();
                    removeChild(target);
                    if (cycle >= 5) {
                        measured++;
                        if (!owned.gc) activeAfterClose++;
                        if (!foreign.gc) foreignActive++;
                    }
                    retired.push({ sprite: target, closeX: target.x });
                    // Reopen immediately with a new manager and display object.
                    if (cycle < 34) openNext();
                }
                if (phase == 0) {
                    var row:Object = retired[cycle];
                    if (cycle >= 5 && row.sprite.x != row.closeX) movedAfterClose++;
                    ++cycle;
                }
            }
            // A full tween duration plus at least 60 ENTER_FRAME events permits
            // TweenLite's periodic sweep to remove its collected table entries.
            if (frame == 330) {
                var retained:int = 0;
                var completeOnce:int = 0;
                for (var index:int = 0; index < retired.length; ++index) {
                    var old:Object = retired[index];
                    if (old.sprite in tweens["masterList"]) retained++;
                    if (callbacksByTarget[index] == 1 && old.sprite.alpha == 0.25) completeOnce++;
                }
                trace("cycles=" + measured + " activeAtClose=" + activeAfterClose
                    + " movedAfterClose=" + movedAfterClose + " foreignActive=" + foreignActive);
                trace("foreignDoneIncludingWarmup=" + foreignCompletions + " retainedKeys=" + retained);
                trace("foreignExactlyOnceAtFinalAlpha=" + completeOnce);
                rapidSwitchAndStaleDispose();
                manager = managerType["instance"];
                samples = addFiveStyles(manager);
                manager.changeClassify(1);
            }
            if (frame == 332) {
                var handles:Array = [];
                for each (sprite in samples) handles.push(tweens["masterList"][sprite][0]);
                var before:int = activeCount(handles);
                manager.dispose();
                for each (sprite in samples) removeChild(sprite);
                closedAt = positions(samples);
                trace("styleTypesActiveBefore=" + before + " styleTypesActiveAfter=" + activeCount(handles));
            }
            if (frame == 336) {
                var moved:int = 0;
                var observed:Array = positions(samples);
                for (var i:int = 0; i < samples.length; ++i) {
                    if (closedAt[i] != observed[i]) moved++;
                }
                trace("styleTypesMovedAfterClose=" + moved);
                removeEventListener(Event.ENTER_FRAME, tick);
                fscommand("quit");
            }
        }

        private function openNext():void {
            manager = managerType["instance"];
            target = new Sprite();
            addChild(target);
            manager.joinSimple(target, 0, 1);
            manager.changeClassify(1);
            owned = tweens["masterList"][target][0];
            // This independent owner animates alpha on the same display object.
            var id:int = callbacksByTarget.length;
            callbacksByTarget.push(0);
            foreign = tweens["to"](target, 0.8, { alpha:0.25, overwrite:0,
                onComplete:foreignDone, onCompleteParams:[id] });
        }

        private function foreignDone(id:int):void {
            foreignCompletions++;
            callbacksByTarget[id]++;
        }

        // Exercise simultaneous owned handles even if the ambient game
        // normally overwrites the preceding tween. Restore the tween policy.
        private function rapidSwitchAndStaleDispose():void {
            var sprite:Sprite = new Sprite();
            var oldManager:Object = managerType["instance"];
            oldManager.joinSimple(sprite, 0, 1);
            var policy:Object = tweens["overwriteManager"];
            var previousMode:int = policy.mode;
            policy.mode = 0;
            oldManager.changeClassify(1);
            oldManager.changeClassify(0);
            oldManager.changeClassify(1);
            policy.mode = previousMode;
            var handles:Array = tweens["masterList"][sprite].concat();
            var before:int = activeCount(handles);
            var other:Object = tweens["to"](sprite, 1, {alpha:0.3, overwrite:0});
            oldManager.dispose();
            var after:int = activeCount(handles);
            var reopened:Object = managerType["instance"];
            oldManager.dispose();
            trace("rapidOwnedBefore=" + before + " rapidOwnedAfter=" + after
                + " foreignPreserved=" + !other.gc + " reopenedSurvived=" + (managerType["instance"] === reopened));
            var lateSimple:Sprite = new Sprite();
            var lateNormal:Sprite = new Sprite();
            oldManager.joinSimple(lateSimple, 0, 1);
            oldManager.joinNormals(lateNormal, 0, 4, new Point(10, 20));
            oldManager.init();
            oldManager.changeClassify(1);
            var staleMoves:int = int(lateSimple in tweens["masterList"])
                + int(lateNormal in tweens["masterList"]);
            trace("lateRegistrationTargets=" + staleMoves);
        }

        private function activeCount(handles:Array):int {
            var active:int = 0;
            for each (var item:Object in handles) {
                if (!item.gc) ++active;
            }
            return active;
        }

        private function addFiveStyles(activeManager:Object):Array {
            var sprites:Array = [];
            for (var i:int = 0; i <= 4; ++i) {
                var sprite:Sprite = new Sprite();
                addChild(sprite);
                sprites.push(sprite);
                // MotionNormal takes a custom destination, the other four use OFF_SET.
                activeManager.joinNormals(sprite, 0, i, new Point(10, 20));
            }
            return sprites;
        }

        private function positions(sprites:Array):Array {
            var pairs:Array = [];
            for each (var sprite:Sprite in sprites) pairs.push(sprite.x + "," + sprite.y);
            return pairs;
        }
    }
}
