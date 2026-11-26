import flash.system.ApplicationDomain;

// The control module can live in a child domain. Its public dependencies resolve there.
try {
    var here:ApplicationDomain = ApplicationDomain.currentDomain;
    if (here.hasDefinition("aola.tools.MyProGameUi")) {
        var bridge:Object = here.getDefinition("aola.tools.MyProGameUi");
        bridge.bindControl(here);
    }
} catch (ignored:Error) {}
