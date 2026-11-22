import java.nio.file.Files;
import java.nio.file.Path;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitClass;

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
            ext.name_index = c.getStringId("BattleHost", true);
            int helper = abc.findClassByName("mmo.petfightnew.common.util.GQWanyuZhichaoEffectHelper");
            abc.script_info.removeIf(script -> script.traits.traits.stream().noneMatch(
                t -> t instanceof TraitClass cls && cls.class_info == helper));
            if (abc.script_info.size() != 1 || abc.script_info.getFirst().traits.traits.size() != 1)
                throw new IllegalArgumentException("Could not isolate the helper's class script");
            // Ruffle links class descriptors even for unexported scripts. Keep just
            // this descriptor and adjust the synthetic movie's class-definition
            // instruction. All business method bodies retain their original bytes.
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
            var statics = abc.class_info.get(helper);
            abc.instance_info.clear();
            abc.instance_info.add(instance);
            abc.class_info.clear();
            abc.class_info.add(statics);
            abc.saveToStream(output);
        }
    }
}
