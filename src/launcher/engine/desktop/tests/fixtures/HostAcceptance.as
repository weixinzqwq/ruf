package {
    import flash.display.Sprite;
    import flash.display.StageAlign;
    import flash.display.StageScaleMode;
    import flash.events.Event;
    import flash.events.MouseEvent;
    import flash.external.ExternalInterface;
    import flash.text.TextField;
    import flash.text.TextFieldType;
    import flash.text.TextFormat;
    import flash.utils.ByteArray;
    import flash.utils.setTimeout;
    [SWF(width="1000", height="850", frameRate="30", backgroundColor="#f4f6fa")]
    public class HostAcceptance extends Sprite {
        private var report:Object = {fixture:"native-host-offline",checks:{},events:[]};
        private var label:TextField = new TextField();
        private var input:TextField = new TextField();
        private var web:String;
        private var fixtureUrl:String;
        public function HostAcceptance() {
            stage.align = StageAlign.TOP_LEFT;
            stage.scaleMode = StageScaleMode.NO_SCALE;
            ExternalInterface.addCallback("hostDispatch", dispatch);
            var previous:* = call("storage.readText", "acceptance.json");
            if (call("app.runtimeEpoch") > 0 && previous) report = JSON.parse(previous);
            label.defaultTextFormat = new TextFormat("Microsoft YaHei",16,0x172235);
            label.width=970; label.height=42; label.x=16; label.y=10; addChild(label);
            input.defaultTextFormat = new TextFormat("Microsoft YaHei",18,0x172235);
            input.type=TextFieldType.INPUT; input.border=true; input.background=true;
            input.x=16; input.y=58; input.width=550; input.height=32; addChild(input);
            input.addEventListener(Event.CHANGE,function(e:Event):void { check("asTextInput",input.text=="奥奇恢复 abc 123",input.text); });
            var button:Sprite=new Sprite();button.graphics.beginFill(0xcbdcf8);button.graphics.drawRoundRect(0,0,190,38,8);button.graphics.endFill();button.x=620;button.y=54;
            var caption:TextField=new TextField();caption.defaultTextFormat=label.defaultTextFormat;caption.text="重启运行时测试";caption.width=185;caption.height=38;caption.mouseEnabled=false;button.addChild(caption);addChild(button);
            button.addEventListener(MouseEvent.CLICK,function(e:MouseEvent):void {call("app.stageLoginHandoff",'{"fixture":"restart"}');call("app.restart");});
            fixtureUrl=loaderInfo.parameters.fixtureUrl;
            check("externalAvailable",ExternalInterface.available);
            check("runtimeVersion",String(call("app.runtimeVersion")).length>0);
            if (loaderInfo.parameters.expectRecovery=="true") {
                var recovery:Object=call("shell.inspect").recovery;
                check("crashRecovery",recovery.safeMode===true && recovery.previousUnclean===true,recovery);
            }
            if(call("app.runtimeEpoch")>0) {
                check("runtimeRestart",true);
                check("handoffOnce",call("app.takeLoginHandoff")=='{"fixture":"restart"}' && call("app.takeLoginHandoff")==null);
            }
            call("storage.writeText","fixture/text.txt","中文 abc");
            call("storage.writeText","fixture/text.txt"," 123",true);
            check("textAppend",call("storage.readText","fixture/text.txt")=="中文 abc 123");
            call("storage.write","fixture/bytes.bin","AP8BAgM=");
            check("binaryStorage",call("storage.read","fixture/bytes.bin")=="AP8BAgM=");
            call("storage.copy","fixture/text.txt","fixture/copy.txt");
            call("storage.move","fixture/copy.txt","fixture/moved.txt");
            check("copyMove",call("storage.readText","fixture/moved.txt")=="中文 abc 123");
            call("storage.remove","fixture/moved.txt");
            check("remove",call("storage.readText","fixture/moved.txt")==null);
            check("embeddedResource",String(call("storage.resourceText","config/native-build.json")).indexOf("{")>=0);
            check("processMemory",Number(call("shell.processMemory").workingSet)>0);
            check("renderer",call("shell.rendererMemory").available===true);
            var protocol:Object=JSON.parse(call("core.call",'{"protocol":1,"op":"core.protocol","args":[]}'));
            check("helperProtocol",protocol.value===1);
            try {
                var bytes:ByteArray=new ByteArray();bytes.writeUTFBytes("abc");bytes.position=1;
                check("nativeBase64",Object(bytes)["aoqiEncodeBase64"](0,0)=="YWJj" && bytes.position==1);
                check("nativeSHA256",Object(bytes)["aoqiHashBytes"]()=="ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
                check("nativeMD5",Object(bytes)["aoqiMD5"]()=="900150983cd24fb0d6963f7d28e17f72");
                Object(bytes)["aoqiDecodeBase64"]("Y W!Jj=");
                check("nativeBase64Decode",bytes.readUTFBytes(bytes.length)=="abc");
            } catch(error:Error) { check("nativeByteArray",false,error.message); }
            web=call("web.create");
            check("webCreate",web!=null && web.length>0);
            call("web.frame",web,0,110,stage.stageWidth,stage.stageHeight-110);
            call("web.load",web,fixtureUrl);
            stage.addEventListener(Event.RESIZE,function(e:Event):void {call("web.frame",web,0,110,stage.stageWidth,stage.stageHeight-110);check("resize",true);});
            call("app.showWindow");call("app.activateWindow");
        }
        private function call(name:String,...args):* {return ExternalInterface.call.apply(null,[name].concat(args));}
        private function check(name:String,passed:Boolean,detail:*=null):void {
            report.checks[name]={passed:passed,detail:detail};
            var good:int=0,total:int=0;for each(var item:Object in report.checks){total++;if(item.passed)good++;}
            label.text="原生宿主离线验收："+good+" / "+total+" 通过。下方第一行测试 Flash 输入；网页区域测试 WebView2。";
            call("storage.writeText","acceptance.json",JSON.stringify(report));
        }
        private function dispatch(channel:String,event:String,payload:*):* {
            if(channel=="app") { if(event=="terminating")check("terminating",true);return true; }
            if(channel!="web:"+web)return null;
            report.events.push({event:event,payload:payload});
            if(event=="navigating" && String(payload).indexOf("navigation=cancel")>=0){check("synchronousNavigationCancel",true);return true;}
            if(event=="loaded") {
                check("webLoaded",true);check("webUrl",String(call("web.url",web)).indexOf("webview-bridge.html")>=0);
                call("web.run",web,"window.aoqiFixture.markRun('native-run-ok');chrome.webview.addEventListener('message',e=>{if(e.data==='native-post-ok')chrome.webview.postMessage({type:'fixture.post',value:e.data})})");
                setTimeout(function():void {call("web.post",web,"native-post-ok");call("web.load",web,fixtureUrl+"?navigation=cancel");},250);
            }
            if(event=="failed")check("webFailedCallback",true,payload);
            if(event=="message") {
                var message:Object=payload is String?JSON.parse(String(payload)):payload;
                if(message.type=="fixture.ready")check(message.wireType=="string"?"webStringIpc":"webObjectIpc",true);
                if(message.type=="fixture.run")check("nativeRun",message.value=="native-run-ok");
                if(message.type=="fixture.post")check("nativePost",message.value=="native-post-ok");
                if(message.type=="fixture.echo") {call("web.post",web,JSON.stringify(message));check("manualEcho-"+message.wireType,true);}
                if(message.type=="fixture.input") {check("webTextInput",message.text=="奥奇恢复 abc 123",message.text);check("webScroll",message.scrollTop>0,message.scrollTop);}
            }
            return false;
        }
    }
}
