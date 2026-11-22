package {
    import flash.display.Sprite;
    import flash.utils.Timer;
    import flash.utils.getDefinitionByName;
    import probe.NoticeHost;

    public class NoticeProbe extends Sprite {
        public function NoticeProbe() {
            var randomType:Class = getDefinitionByName("mmo.MessageSend.MessageRandomShow") as Class;
            var serviceType:Class = getDefinitionByName("mmo.MessageSend.MessageSendService") as Class;
            var titleType:Class = getDefinitionByName("mmo.interfaces.title.TitleEvent") as Class;
            var eventType:Class = getDefinitionByName("mmo.framework.comm.SocketClientEvent") as Class;
            var random:Object = new randomType();
            var service:Object = new serviceType();
            NoticeHost.hide = true;
            random.initSystemdMsg();
            trace("random init", random.sTimer == null, random.showTimer == null);
            random.sTimer = new Timer(180000);
            random.onSystemMsgComes(null);
            trace("random tick", random.sTimer.running);
            random.sTimer.stop();

            service.onHonorMsgDispatch(null);
            service.onTitleHandle(null);
            service.onReturnBroadcast(null);
            service.messageComing(null);
            trace("empty", service.messages.length, NoticeHost.calls.join(","));
            service.messageShowing = true;
            service.messages.push("prior-1", "prior-2");
            service.messageComing(null);
            trace("clear", service.messages.length, service.messageShowing, NoticeHost.calls.pop());

            NoticeHost.hide = false;
            service.onTitleHandle(new titleType("title", {str:"body"}));
            service.onReturnBroadcast(new eventType("count", {tn:2500}));
            service.messageComing(null);
            trace("restored", service.messages.length, service.messages[0].type, service.messages[0].boxBody,
                  service.messages[1] === service.messages[3], NoticeHost.calls.slice(-3).join(","));
        }
    }
}

package probe {
    public class NoticeHost {
        public static var hide:Boolean;
        public static var calls:Array = [];
        public static function call(name:String, channel:String):Boolean {
            if (name != "aolaHideNotice") throw new Error(name);
            calls.push(channel);
            return hide;
        }
    }
}

package mmo.framework.comm {
    import flash.events.Event;
    public class SocketClientEvent extends Event {
        public var params:Object;
        public function SocketClientEvent(type:String, params:Object) {
            super(type);
            this.params = params;
        }
    }
}
