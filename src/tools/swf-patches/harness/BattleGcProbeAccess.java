import java.io.*;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.*;
import com.jpexs.decompiler.flash.abc.types.traits.*;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class BattleGcProbeAccess {
    static byte[] op(int code, int... args) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[code], args).getBytes();
    }

    // Copy an original instruction block only if all its opcodes match.
    static void copy(ByteArrayOutputStream out, List<AVM2Instruction> ops, int start, int... codes) {
        for (int offset = 0; offset < codes.length; offset++) {
            var inst = ops.get(start + offset);
            if (inst.definition.instructionCode != codes[offset]) throw new IllegalStateException("Unexpected dispose body");
            out.writeBytes(inst.getBytes());
        }
    }

    public static void main(String[] args) throws Exception {
        try (var input = Files.newInputStream(Path.of(args[0])); var output = Files.newOutputStream(Path.of(args[1]))) {
            var abc = new SWF(input, false).getAbcList().getFirst().getABC();
            var c = abc.constants;
            int owner = abc.findClassByName("mmo.petfightnew.common.FightPanel");
            if (owner != 720 || abc.instance_info.get(owner).iinit_index != 4023) throw new IllegalStateException("FightPanel changed");
            var panel = abc.instance_info.get(owner);
            var names = Set.of("_isDisposed", "disposeManagers", "dispose");
            panel.instance_traits.traits.removeIf(t -> !names.contains(c.getString(t.getName(abc).name_index)));
            if (panel.instance_traits.traits.size() != 3) throw new IllegalStateException("Missing panel state or methods");
            var ids = new HashMap<String,Integer>();
            for (var t : panel.instance_traits.traits) {
                String name = c.getString(t.getName(abc).name_index);
                ids.put(name, t.name_index);
                if (t instanceof TraitMethodGetterSetter m && m.method_info != (name.equals("dispose") ? 4167 : 4073))
                    throw new IllegalStateException("Unexpected method index");
            }
            // Preserve the complete namespace, owner index and method bindings. Stub all
            // unrelated class dependencies because Ruffle links even unexported classes.
            abc.instance_info.subList(owner + 1, abc.instance_info.size()).clear();
            abc.class_info.subList(owner + 1, abc.class_info.size()).clear();
            for (int i = 0; i <= owner; i++) {
                var instance = abc.instance_info.get(i);
                var cls = abc.class_info.get(i);
                cls.static_traits.traits.clear();
                instance.interfaces = new int[0];
                if (i == owner) {
                    abc.findBody(cls.cinit_index).setCodeBytes(op(ReturnVoid));
                } else {
                    instance.flags = InstanceInfo.CLASS_SEALED;
                    instance.super_index = c.getPublicQnameId("Object", false);
                    instance.instance_traits.traits.clear();
                }
            }
            abc.script_info.removeIf(s -> s.traits.traits.stream().noneMatch(t -> t instanceof TraitClass def && def.class_info == owner));
            if (abc.script_info.size() != 1 || abc.script_info.getFirst().traits.traits.size() != 1)
                throw new IllegalStateException("Unexpected panel script");
            abc.findBody(4023).setCodeBytes(new byte[]{(byte)GetLocal0,(byte)PushScope,(byte)GetLocal0,(byte)ConstructSuper,0,(byte)ReturnVoid});
            abc.findBody(4073).setCodeBytes(op(ReturnVoid));
            var dispose = abc.findBody(4167);
            var ops = dispose.getCode().code;
            var out = new ByteArrayOutputStream();
            copy(out, ops, 0, GetLocal0, PushScope);
            int chunks = 0;
            for (int i = 0; i < ops.size(); i++) {
                var inst = ops.get(i);
                int code = inst.definition.instructionCode;
                if (code == GetProperty && inst.operands[0] == ids.get("_isDisposed")) {
                    copy(out, ops, i - 1, GetLocal0, GetProperty, IfFalse, ReturnVoid);
                    if (ops.get(i + 1).operands[0] != 1) throw new IllegalStateException("Guard no longer skips only returnvoid");
                    chunks++;
                }
                if (code == InitProperty && inst.operands[0] == ids.get("_isDisposed")) {
                    copy(out, ops, i - 2, GetLocal0, PushTrue, InitProperty);
                    chunks++;
                }
                if (code == CallPropVoid && inst.operands[0] == ids.get("disposeManagers")) {
                    copy(out, ops, i - 1, GetLocal0, CallPropVoid);
                    if (inst.operands[1] != 0) throw new IllegalStateException("Unexpected disposeManagers arguments");
                    chunks++;
                }
            }
            if (chunks != 3) throw new IllegalStateException("Incomplete original dispose path");
            out.writeBytes(op(GetLex, c.getQnameId("BattleTail", Namespace.KIND_PACKAGE, "probe", true)));
            out.writeBytes(op(CallPropVoid, c.getPublicQnameId("afterManagers", true), 0));
            copy(out, ops, ops.size() - 1, ReturnVoid);
            dispose.setCodeBytes(out.toByteArray());
            abc.saveToStream(output);
        }
    }
}
