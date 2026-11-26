package mmo.checkversion
{
   import flash.display.Loader;
   import flash.display.LoaderInfo;
   import flash.display.MovieClip;
   import flash.display.Sprite;
   import flash.events.Event;
   import flash.events.IOErrorEvent;
   import flash.events.KeyboardEvent;
   import flash.external.ExternalInterface;
   import flash.media.Sound;
   import flash.net.URLLoader;
   import flash.net.URLLoaderDataFormat;
   import flash.net.URLRequest;
   import flash.system.ApplicationDomain;
   import flash.system.LoaderContext;
   import flash.text.StyleSheet;
   import flash.text.TextField;
   import flash.text.TextFieldAutoSize;
   import flash.ui.ContextMenu;
   import flash.utils.ByteArray;

   public class Load extends Sprite
   {

      private static const LAUNCHER_EXTENSIONS:Object = {
         "StartupExtension":true,
         "UserExtension":true,
         "TeamExtension":true,
         "BuddyManagerExtension":true,
         "ScoreManager":true,
         "Log":true,
         "SurveyExtension":true,
         "PlayBookExtension":true,
         "OneOffExtension":true,
         "MemberExtension":true,
         "GameRealTimeRankManager":true,
         "GameMessageList":true,
         "UserDisplayExtension":true,
         "PocketMonsterExtension":true,
         "PetFightExtension":true,
         "PetExtension":true,
         "PetExtension2":true,
         "PetEggExtension":true,
         "AntiAddictionExtension":true,
         "TowerExtension":true,
         "SynthesizeExtension":true,
         "MaterialExtension":true,
         "RobotExtension":true,
         "MatchExtension":true,
         "FightRingExtension":true,
         "RobMedalArenaExtension":true,
         "HouseExtension":true,
         "UserFurntitureInfoExtension":true,
         "ShootExtension":true,
         "TaskExtension":true,
         "DailyTaskExtension":true,
         "GiftBagExtension":true,
         "AvatarExtension":true,
         "FollowExtension":true,
         "SundriesExtension":true,
         "FBExtension":true,
         "AvocationExtension":true,
         "MidautumnGameExtension":true,
         "MoveStateExtension":true,
         "UserStateExtension":true,
         "AchievementExtension":true,
         "NewAreaExtension":true,
         "HolidayExtension":true,
         "MailBoxExtension":true,
         "PTowerExtension":true,
         "PMDailyCollectExtension":true,
         "VipNickNameExtension":true,
         "VipTowerExtension":true,
         "PlantExtension":true,
         "PlantItemExtension":true,
         "ConnectTestExtension":true,
         "WisdomStarExtension":true,
         "DonateExtension":true,
         "BalloonExtension":true,
         "UserGameExtension":true,
         "PlaneChessGameManager":true,
         "PeriodicActiveExtension":true,
         "MagicCubicExtension":true,
         "WarTeamExtension":true,
         "WTBeatBossExtension":true,
         "HouseOutSideExtension":true,
         "WarTeamTaskExtension":true,
         "PiracyTowerExtension":true,
         "MentalHallExtension":true,
         "ZooExtension":true,
         "StateExtension":true,
         "HotSpringExtension":true,
         "MSExtension":true,
         "LabExtension":true,
         "KOFExtension":true,
         "FieldGameExtension":true,
         "HbhBeatBossExtension":true,
         "TeamTowerExtension":true,
         "LuandouExtension":true,
         "Sundries2ndExtension":true,
         "GalaxyExtension":true,
         "PetEquipExtension":true,
         "BattleGodExtension":true,
         "CorpsExtension":true,
         "SpeedTestExtension":true,
         "LightWingExtension":true,
         "XSystemExtension":true,
         "StarBossExtension":true,
         "FiveYearPKExtension":true,
         "KaiSaExtension":true,
         "BurningCrusadeExtension":true,
         "TrainingExtension":true
      };

      private function registerLauncherBridge() : void
      {
         if(ExternalInterface.available)
         {
            ExternalInterface.addCallback("aolaSendCommand",this.onLauncherSend);
            ExternalInterface.addCallback("aolaBridgeReady",this.onLauncherReady);
         }
      }

      private function onLauncherReady() : String
      {
         if(!ApplicationDomain.currentDomain.hasDefinition("mmo.play.actthings.data.ActClientSender"))
            return "BridgeNotReady";
         return "AolaLauncherBridge1";
      }

      private function onLauncherSend(requestId:String, extension:String, command:String, parametersJson:String) : String
      {
         if(requestId == null || !/^[a-f0-9]{32}$/.test(requestId) ||
            !LAUNCHER_EXTENSIONS.hasOwnProperty(extension) || command == null ||
            command.length == 0 || command.length > 256 ||
            command != command.replace(/^\s+|\s+$/g,"") || parametersJson == null ||
            parametersJson.length == 0 || parametersJson.length > 262144)
            return "FlashRejected";
         if(onLauncherReady() != "AolaLauncherBridge1") return "BridgeNotReady";
         try
         {
            var parameters:Object = JSON.parse(parametersJson);
            if(parameters == null || parameters is Array || typeof parameters != "object")
               return "FlashRejected";
            delete parameters[":ext_seq;"];
            var sender:Object = ApplicationDomain.currentDomain.getDefinition("mmo.play.actthings.data.ActClientSender");
            sender["sendMessage"](extension,command,parameters,function(response:Object):void
            {
               onLauncherResponse(requestId,response);
            },false);
            return "SubmittedToFlash";
         }
         catch(error:Error)
         {
            return "FlashRejected";
         }
      }

      private function onLauncherResponse(requestId:String, response:Object) : void
      {
         // A callback with no explicit result code is not server success.
         var status:String = "ServerResponse";
         var code:String = "";
         if(response != null)
         {
            var value:* = undefined;
            if(response.hasOwnProperty("r")) value = response.r;
            else if(response.hasOwnProperty("suc")) value = response.suc;
            else if(response.hasOwnProperty("res")) value = response.res;
            if(value !== undefined && value !== null && String(value).match(/^-?\d+$/) != null)
            {
               code = String(value);
               status = code == "1" ? "ServerSucceeded" : "ServerRejected";
            }
         }
         try
         {
            if(ExternalInterface.available)
               ExternalInterface.call("aolaCommandCompleted",requestId,status,code);
         }
         catch(error:Error)
         {
         }
      }


      internal var mc:MovieClip;

      internal var txt:TextField;

      private const showMemory:Boolean = false;

      private const showCode:String = "showmethememory";

      private var startGetCode:Boolean;

      private var code:String;

      public var version:XML;

      public var loaderSwfBtyes:ByteArray;

      public function Load()
      {
         var _loc2_:ContextMenu = null;
         super();
         this.registerLauncherBridge();
         if(this.stage.loaderInfo.url.indexOf("www.mygame.com") < 0)
         {
            _loc2_ = new ContextMenu();
            _loc2_.hideBuiltInItems();
            this.contextMenu = _loc2_;
         }
         var _loc1_:Sound = new Sound(new URLRequest(""));
         _loc1_.play();
         _loc1_.close();
         stage.stageFocusRect = false;
         this.loadVersion();
         this.addEventListener(Event.ADDED_TO_STAGE,this.onAddedToStage);
      }

      private function onAddedToStage(param1:Event) : void
      {
         this.removeEventListener(Event.ADDED_TO_STAGE,this.onAddedToStage);
         var _loc2_:TextField = new TextField();
         var _loc3_:StyleSheet = new StyleSheet();
         _loc3_.setStyle("a:hover",{"textDecoration":"underline"});
         _loc3_.setStyle("a:link",{"color":"#0000FF"});
         _loc2_.styleSheet = _loc3_;
         var _loc4_:String = "即将抵达奥拉星...";
         _loc2_.htmlText = _loc4_;
         _loc2_.autoSize = TextFieldAutoSize.LEFT;
         _loc2_.x = (960 - _loc2_.width) * 0.5;
         _loc2_.y = (560 - _loc2_.height) * 0.5;
         _loc2_.name = "start";
         _loc2_.selectable = false;
         stage.addChild(_loc2_);
      }

      private function loadVersion() : void
      {
         var _loc1_:URLRequest = null;
         var _loc3_:Date = null;
         if(this.stage.loaderInfo.url.indexOf("file://") == 0)
         {
            _loc1_ = new URLRequest("start.xml");
         }
         else
         {
            _loc3_ = new Date();
            _loc1_ = new URLRequest("start~" + _loc3_.valueOf().toString() + ".xml");
         }
         var _loc2_:URLLoader = new URLLoader(_loc1_);
         _loc2_.addEventListener(Event.COMPLETE,this.onLoadVersion);
         _loc2_.addEventListener(IOErrorEvent.IO_ERROR,this.onLoadVersionError);
      }

      private function jsGetValue(param1:String, param2:*, ... rest) : *
      {
         var result:* = undefined;
         var functionName:String = param1;
         var defaultValue:* = param2;
         var arguments:Array = rest;
         try
         {
            if(!ExternalInterface.available)
            {
               return defaultValue;
            }
            result = ExternalInterface.call(functionName,arguments);
            return result == undefined ? defaultValue : result;
         }
         catch(errObject:Error)
         {
            return defaultValue;
         }
      }

      private function getTestCode() : String
      {
         var _loc1_:String = null;
         try
         {
            if(!ExternalInterface.available)
            {
               return null;
            }
            return ExternalInterface.call("getQS","testf");
         }
         catch(errObject:Error)
         {
         }
         return null;
      }

      private function onLoadVersionError(param1:IOErrorEvent) : void
      {
         this.loadVersion();
      }

      private function onLoadVersion(param1:Event) : void
      {
         this.version = XML(URLLoader(param1.target).data);
         if(this.version.t != "open" && this.getTestCode() == null)
         {
            if(this.version.v.length > 0)
            {
               this.load(String(this.version.t) + "~" + this.version.v + ".swf");
            }
            else
            {
               this.load(String(this.version.t) + ".swf");
            }
            stage.getChildByName("start").visible = false;
            return;
         }
         var _loc2_:String = "";
         if(this.version.lv != undefined)
         {
            _loc2_ = this.version.lv;
         }
         else if(this.version.v != undefined)
         {
            _loc2_ = this.version.v;
         }
         if(_loc2_.length > 0)
         {
            this.load("loader" + "~" + _loc2_ + ".swf");
         }
         else
         {
            this.load("loader.swf");
         }
      }

      private function load(param1:String) : void
      {
         var _loc2_:URLRequest = new URLRequest(param1);
         var _loc3_:URLLoader = new URLLoader();
         _loc3_.dataFormat = URLLoaderDataFormat.BINARY;
         _loc3_.addEventListener(Event.COMPLETE,this.onURLLoadSuccess);
         _loc3_.load(_loc2_);
      }

      private function onURLLoadSuccess(param1:Event) : void
      {
         var _loc2_:URLLoader = param1.target as URLLoader;
         _loc2_.removeEventListener(Event.COMPLETE,this.onURLLoadSuccess);
         this.loaderSwfBtyes = _loc2_.data;
         var _loc3_:Loader = new Loader();
         _loc3_.contentLoaderInfo.addEventListener(Event.COMPLETE,this.onLoadSuccess);
         _loc3_.loadBytes(this.loaderSwfBtyes,new LoaderContext(false,ApplicationDomain.currentDomain));
      }

      private function onLoadSuccess(param1:Event) : void
      {
         var _loc4_:MemoryWatch = null;
         var _loc2_:LoaderInfo = param1.target as LoaderInfo;
         _loc2_.removeEventListener(Event.COMPLETE,this.onLoadSuccess);
         var _loc3_:TextField = stage.getChildByName("start") as TextField;
         if(Boolean(_loc3_) && Boolean(_loc3_.parent))
         {
            _loc3_.parent.removeChild(_loc3_);
         }
         this.addChild(Sprite(_loc2_.content));
         if(this.showMemory)
         {
            _loc4_ = new MemoryWatch();
            this.addChild(_loc4_);
         }
         else
         {
            this.stage.addEventListener(KeyboardEvent.KEY_DOWN,this.onKeyInput);
         }
      }

      private function onKeyInput(param1:KeyboardEvent) : void
      {
         var _loc2_:MemoryWatch = null;
         if(param1.ctrlKey && param1.altKey)
         {
            this.startGetCode = true;
            this.code = "";
         }
         else if(this.startGetCode)
         {
            this.code += String.fromCharCode(param1.charCode);
            if(this.code.length == this.showCode.length)
            {
               if(this.code == this.showCode)
               {
                  _loc2_ = new MemoryWatch();
                  this.addChild(_loc2_);
                  this.stage.removeEventListener(KeyboardEvent.KEY_DOWN,this.onKeyInput);
               }
               else
               {
                  this.startGetCode = false;
               }
            }
         }
      }
   }
}
