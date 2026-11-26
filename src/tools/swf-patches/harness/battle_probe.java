import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Set;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitClass;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

// Only export the real helper into the isolated test movie. The full original
// method, including its activation traits and nested closures, stays intact.
class BattleProbeAccess {
    public static void main(String[] args) throws Exception {
        try (var input = Files.newInputStream(Path.of(args[0]));
             var output = Files.newOutputStream(Path.of(args[1]))) {
            var abc = new SWF(input, false).getAbcList().getFirst().getABC();
            var c = abc.constants;
            var ext = c.getMultiname(c.getQnameId("ExternalInterface", Namespace.KIND_PACKAGE, "flash.external", false));
            ext.namespace_index = c.getNamespaceId(Namespace.KIND_PACKAGE, "probe", 0, true);
            boolean portraits = args.length == 3 && args[2].equals("portraits");
            boolean backdrop = args.length == 3 && Set.of("StartMovie", "StartMovie3v3").contains(args[2]);
            ext.name_index = c.getStringId(backdrop ? "BackdropHost" : portraits ? "PortraitHost" : "BattleHost", true);
            int helper = abc.findClassByName(backdrop ? "mmo.petfightnew.common.startmovie." + args[2] : portraits
                ? "mmo.petfightnew.common.fightcontroller.ChangePetView"
                : "mmo.petfightnew.common.util.GQWanyuZhichaoEffectHelper");
            abc.script_info.removeIf(script -> script.traits.traits.stream().noneMatch(
                t -> t instanceof TraitClass cls && cls.class_info == helper));
            if (abc.script_info.size() != 1 || abc.script_info.getFirst().traits.traits.size() != 1)
                throw new IllegalArgumentException("Could not isolate the helper's class script");
            // Ruffle links class descriptors even for unexported scripts. Keep just
            // this descriptor and adjust the synthetic movie's class-definition
            // instruction. Methods under test keep their original bodies.
            var script = abc.script_info.getFirst();
            var definition = (TraitClass) script.traits.traits.getFirst();
            definition.class_info = 0;
            var initializer = abc.findBody(script.init_index);
            var code = initializer.getCode();
            int definitions = 0;
            for (var op : code.code) {
                if (op.definition.instructionName.equals("newclass") && op.operands[0] == helper) {
                    op.operands[0] = 0;
                    definitions++;
                }
            }
            if (definitions != 1) throw new IllegalArgumentException("Unexpected helper initializer");
            initializer.setCode(code);
            var instance = abc.instance_info.get(helper);
            if (backdrop) {
                // Start at the end of loading. Avoid the normal constructor's
                // game-service startup, retaining enterFight, dispose and their closures.
                abc.findBody(instance.iinit_index).setCodeBytes(new byte[]{
                    (byte)GetLocal0, (byte)PushScope, (byte)GetLocal0, (byte)ConstructSuper, 0, (byte)ReturnVoid});
                var visible = Set.of("enterFight", "timer", "startMovie");
                var internal = Set.of("dispose", "onTimesup", "onOthersLoadDone", "onEscape", "sendLoad",
                        "myTeamInfo", "vsTeamInfo", "frameTaskList", "_mc", "_mcRole");
                instance.instance_traits.traits.removeIf(t -> {
                    var name = t.getName(abc);
                    var value = c.getString(name.name_index);
                    if (visible.contains(value)) {
                        name.namespace_index = c.getNamespaceId(Namespace.KIND_PACKAGE, "", 0, false);
                        return false;
                    }
                    return !internal.contains(value);
                });
            }
            if (portraits) {
                // Expose only the fields needed to prepare the already-loaded model
                // and the entry methods under test. Original method code is untouched.
                var visible = Set.of("_locaId", "_petContainer", "_isFace", "showYiRenMovie",
                    "showJiuRiShenAppearMovie", "showJuexingAppearMovie",
                    "showDengShenChangJieAppearMovie", "showZhenLiFaDianAppearMovie");
                var internal = Set.of("fightPanel", "init", "_pmSvr", "onPetChange", "dispose");
                instance.instance_traits.traits.removeIf(t -> {
                    var name = t.getName(abc);
                    var value = c.getString(name.name_index);
                    if (visible.contains(value)) {
                        name.namespace_index = c.getNamespaceId(Namespace.KIND_PACKAGE, "", 0, false);
                        return false;
                    }
                    return !internal.contains(value);
                });
            }
            var statics = abc.class_info.get(helper);
            abc.instance_info.clear();
            abc.instance_info.add(instance);
            abc.class_info.clear();
            abc.class_info.add(statics);
            abc.saveToStream(output);
        }
    }
}
