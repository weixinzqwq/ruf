package mmo.play.actthings.data
{
   import flash.external.ExternalInterface;
   public class ActClientSender
   {
      public static function sendMessage(extension:String, command:String, parameters:Object,
                                         callback:Function, isNeedLock:Boolean = true) : void
      {
         ExternalInterface.call("fixtureSend", extension, command, JSON.stringify(parameters), isNeedLock);
         if(command == "fixture-success") callback({r:1});
         else if(command == "fixture-rejected") callback({suc:0});
         else callback({});
      }
   }
}
