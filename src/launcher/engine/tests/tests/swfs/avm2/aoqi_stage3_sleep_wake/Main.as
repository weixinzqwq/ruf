package {
import flash.display.MovieClip;
import flash.events.Event;
public class Main extends MovieClip {
    private var roots:Array = [];
    private var leaf:MovieClip;
    private var added:MovieClip;
    private var ticks:int = 0;
    private var broadcasts:int = 0;
    public function Main() {
        stop();
        for (var n:int=0;n<100;n++) {
            var root:MovieClip = new MovieClip(); root.stop(); roots.push(root);
            var parent:MovieClip = root;
            for (var d:int=0;d<8;d++) {
                var child:MovieClip = new MovieClip(); child.stop(); parent.addChild(child); parent=child;
            }
            if (n==0) leaf=parent;
        }
        leaf.addEventListener(Event.ENTER_FRAME, function(e:Event):void { broadcasts++; });
        addEventListener(Event.ENTER_FRAME, step);
        trace("ready");
    }
    private function step(e:Event):void {
        ticks++;
        if (ticks==2) leaf.play();
        if (ticks==3) leaf.stop();
        if (ticks==4) leaf.gotoAndStop(1);
        if (ticks==5) { added=new MovieClip(); added.stop(); leaf.addChild(added); }
        if (ticks==6) leaf.removeChild(added);
        if (ticks==7) addChild(roots[0]);
        if (ticks==8) removeChild(roots[0]);
        trace("tick",ticks,"broadcasts",broadcasts,"children",leaf.numChildren,"parent",roots[0].parent==this);
    }
}
}