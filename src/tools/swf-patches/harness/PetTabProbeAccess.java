import java.io.ByteArrayOutputStream;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.*;
import com.jpexs.decompiler.flash.abc.types.traits.*;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

// Keep the original detail event/listener, tab lifecycle and async completion methods.
// Isolate page layout construction and count calls at the leaf paint boundary.
class PetTabProbeAccess {
    static class Code extends ByteArrayOutputStream {
        void op(int opcode, int... args) {
            writeBytes(new AVM2Instruction(0, AVM2Code.instructionSet[opcode], args).getBytes());
        }
    }
    static Trait named(ABC abc, InstanceInfo cls, String name) {
        return cls.instance_traits.traits.stream().filter(t -> name.equals(
            abc.constants.getString(t.getName(abc).name_index))).findFirst().orElseThrow();
    }
    public static void main(String[] args) throws Exception {
        var abc = new SWF(Files.newInputStream(Path.of(args[0])), false).getAbcList().getFirst().getABC();
        var c = abc.constants;
        var remap = new LinkedHashMap<Integer,Integer>();
        var instances = new ArrayList<InstanceInfo>();
        var classes = new ArrayList<ClassInfo>();
        var pack = "mmo.petpackage.panel.inclusion.petdetailinfo.";
        int base = abc.findClassByName(pack + "PetDetailBase");
        for (String owner : List.of("mmo.petpackage.panel.inclusion.interfaces.IPetView",
                pack + "PetDetailBase", pack + "PetStudyInfo", pack + "PetDetailAbility",
                pack + "PetItem", pack + "GodCard", "mmo.petpackage.panel.SimpleViewController")) {
            int index = abc.findClassByName(owner);
            var cls = abc.instance_info.get(index);
            var statics = abc.class_info.get(index);
            remap.put(index, instances.size());
            instances.add(cls);
            classes.add(statics);
            if (cls.isInterface()) continue;
            if (index == base) {
                named(abc, cls, "setCanUpdate").getName(abc).namespace_index =
                    c.getNamespaceId(Namespace.KIND_PACKAGE, "", 0, false);
                named(abc, cls, "onPetDetailedCached").getName(abc).namespace_index =
                    c.getNamespaceId(Namespace.KIND_PACKAGE, "", 0, false);
                for (String name : List.of("initUI", "refreshUI")) {
                    var body = abc.findBody(((TraitMethodGetterSetter)named(abc, cls, name)).method_info);
                    var code = new Code();
                    code.op(GetLocal0); code.op(PushScope);
                    code.op(GetLex, c.getQnameId("Paint", Namespace.KIND_PACKAGE, "probe", true));
                    code.op(GetLocal0); code.op(GetProperty, named(abc, cls, "pdi").name_index);
                    code.op(CallPropVoid, c.getPublicQnameId(name, true), 1);
                    code.op(ReturnVoid);
                    body.max_stack = 2;
                    body.setCodeBytes(code.toByteArray());
                }
            } else {
                var ctor = abc.findBody(cls.iinit_index);
                var ops = ctor.getCode().code;
                var code = new Code();
                code.op(GetLocal0); code.op(PushScope); code.op(GetLocal0); code.op(ConstructSuper, 0);
                // Keep the actual opt-in write when it exists. Original/control do not opt in.
                if (ops.size() >= 5 && ops.get(ops.size() - 2).definition.instructionCode == SetProperty &&
                    "aolaDetailTabMode".equals(c.getString(c.getMultiname(ops.get(ops.size() - 2).operands[0]).name_index))) {
                    if (ops.get(ops.size() - 5).definition.instructionCode != GetLocal0 ||
                        ops.get(ops.size() - 4).definition.instructionCode != GetProperty ||
                        !"onPetDetailedCached".equals(c.getString(c.getMultiname(ops.get(ops.size() - 4).operands[0]).name_index)) ||
                        ops.get(ops.size() - 3).definition.instructionCode != PushByte ||
                        ops.get(ops.size() - 3).operands[0] != 1)
                        throw new IllegalArgumentException("Unexpected tab registration in " + owner);
                    for (int k = ops.size() - 5; k < ops.size() - 1; k++) code.writeBytes(ops.get(k).getBytes());
                }
                code.op(ReturnVoid);
                ctor.setCodeBytes(code.toByteArray());
                cls.instance_traits.traits.clear();
                cls.interfaces = new int[0];
                statics.static_traits.traits.clear();
                abc.findBody(statics.cinit_index).setCodeBytes(new byte[]{(byte)ReturnVoid});
            }
        }
        abc.script_info.removeIf(s -> s.traits.traits.stream().noneMatch(
            t -> t instanceof TraitClass def && remap.containsKey(def.class_info)));
        if (abc.script_info.size() != remap.size()) throw new IllegalArgumentException("Unexpected class scripts");
        for (var script : abc.script_info) {
            var def = (TraitClass)script.traits.traits.getFirst();
            int old = def.class_info;
            def.class_info = remap.get(old);
            var init = abc.findBody(script.init_index);
            var code = init.getCode();
            for (var op : code.code)
                if (op.definition.instructionCode == NewClass && op.operands[0] == old) op.operands[0] = def.class_info;
            init.setCode(code);
        }
        abc.instance_info.clear(); abc.instance_info.addAll(instances);
        abc.class_info.clear(); abc.class_info.addAll(classes);
        try (var output = Files.newOutputStream(Path.of(args[1]))) { abc.saveToStream(output); }
    }
}
