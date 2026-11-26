import java.io.ByteArrayOutputStream;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.*;
import com.jpexs.decompiler.flash.abc.types.traits.*;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

// Retain switchPet's complete original body, private trait and indices. Route
// the public init method to it with an offline panel in place of game services.
class BattleSwitchProbeAccess {
    static byte[] op(int code, int... args) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[code], args).getBytes();
    }

    public static void main(String[] args) throws Exception {
        try (var input = Files.newInputStream(Path.of(args[0])); var output = Files.newOutputStream(Path.of(args[1]))) {
            var abc = new SWF(input, false).getAbcList().getFirst().getABC();
            var c = abc.constants;
            int owner = abc.findClassByName("mmo.petfightnew.common.fightcontroller.SwitchPm");
            var cls = abc.instance_info.get(owner);
            abc.script_info.removeIf(s -> s.traits.traits.stream().noneMatch(t -> t instanceof TraitClass d && d.class_info == owner));
            if (abc.script_info.size() != 1 || abc.script_info.getFirst().traits.traits.size() != 1)
                throw new IllegalArgumentException("Unexpected SwitchPm script");
            var names = Set.of("init", "switchPet", "fightPanel", "curPet", "switchSkill", "onGotPet");
            var keep = new HashMap<String,Trait>();
            for (var t : cls.instance_traits.traits) {
                var name = c.getString(t.getName(abc).name_index);
                if (names.contains(name)) keep.put(name, t);
            }
            if (!keep.keySet().equals(names)) throw new IllegalArgumentException("Missing SwitchPm traits");
            abc.instance_info.subList(owner + 1, abc.instance_info.size()).clear();
            abc.class_info.subList(owner + 1, abc.class_info.size()).clear();
            for (int i = 0; i <= owner; ++i) {
                var instance = abc.instance_info.get(i);
                abc.class_info.get(i).static_traits.traits.clear();
                instance.interfaces = new int[0];
                if (i == owner) {
                    instance.instance_traits.traits.retainAll(keep.values());
                } else {
                    instance.name_index = c.getQnameId("Unused" + i, Namespace.KIND_PACKAGE, "probe.unused", true);
                    instance.flags = InstanceInfo.CLASS_SEALED;
                    instance.super_index = c.getPublicQnameId("Object", false);
                    instance.instance_traits.traits.clear();
                }
            }
            abc.findBody(abc.class_info.get(owner).cinit_index).setCodeBytes(op(ReturnVoid));
            abc.findBody(cls.iinit_index).setCodeBytes(new byte[]{(byte)GetLocal0,(byte)PushScope,(byte)GetLocal0,(byte)ConstructSuper,0,(byte)ReturnVoid});
            var init = abc.findBody(((TraitMethodGetterSetter)keep.get("init")).method_info);
            var out = new ByteArrayOutputStream();
            out.writeBytes(op(GetLocal0)); out.writeBytes(op(PushScope));
            out.writeBytes(op(GetLocal0)); out.writeBytes(op(GetLocal1)); out.writeBytes(op(SetProperty, keep.get("fightPanel").name_index));
            out.writeBytes(op(GetLocal0)); out.writeBytes(op(PushByte, 0)); out.writeBytes(op(CallPropVoid, keep.get("switchPet").name_index, 1));
            out.writeBytes(op(ReturnVoid));
            init.setCodeBytes(out.toByteArray());
            abc.saveToStream(output);
        }
    }
}
