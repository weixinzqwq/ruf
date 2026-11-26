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

// Exercise the game's selected-tab loop, private canonical call site and actual
// detail listener. Replace service initialization and leaf painting with counters.
class PetActiveTabProbeAccess {
    static Trait trait(ABC abc, InstanceInfo owner, String name) {
        return owner.instance_traits.traits.stream().filter(t -> name.equals(
            abc.constants.getString(t.getName(abc).name_index))).findFirst().orElseThrow();
    }
    static MethodBody body(ABC abc, InstanceInfo owner, String name) {
        return abc.findBody(((TraitMethodGetterSetter)trait(abc, owner, name)).method_info);
    }
    static byte[] op(int opcode, int... args) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[opcode], args).getBytes();
    }
    public static void main(String[] args) throws Exception {
        var abc = new SWF(Files.newInputStream(Path.of(args[0])), false).getAbcList().getFirst().getABC();
        var c = abc.constants;
        var remap = new LinkedHashMap<Integer,Integer>();
        var instances = new ArrayList<InstanceInfo>();
        var classes = new ArrayList<ClassInfo>();
        String detail = "mmo.petpackage.panel.inclusion.petdetailinfo.";
        int publicNs = c.getNamespaceId(Namespace.KIND_PACKAGE, "", 0, false);
        for (var owner : List.of("mmo.petpackage.panel.inclusion.interfaces.IPetView",
                detail + "PetDetailBase", detail + "PetDetailSkill", "mmo.petpackage.panel.PetPackagePanel")) {
            int index = abc.findClassByName(owner);
            var cls = abc.instance_info.get(index);
            var statics = abc.class_info.get(index);
            remap.put(index, instances.size());
            instances.add(cls); classes.add(statics);
            if (cls.isInterface()) continue;
            if (owner.endsWith("PetDetailBase")) {
                trait(abc, cls, "setCanUpdate").getName(abc).namespace_index = publicNs;
                for (var name : List.of("initUI", "refreshUI")) {
                    var b = body(abc, cls, name);
                    var out = new ByteArrayOutputStream();
                    out.writeBytes(op(GetLex, c.getQnameId("Paint", Namespace.KIND_PACKAGE, "probe", true)));
                    out.writeBytes(op(GetLocal0)); out.writeBytes(op(GetProperty, trait(abc, cls, "pdi").name_index));
                    out.writeBytes(op(CallPropVoid, c.getPublicQnameId(name, true), 1));
                    out.writeBytes(op(ReturnVoid)); b.setCodeBytes(out.toByteArray()); b.max_stack = 2;
                }
                continue;
            }
            abc.findBody(cls.iinit_index).setCodeBytes(new byte[]{(byte)GetLocal0,(byte)PushScope,
                (byte)GetLocal0,(byte)ConstructSuper,0,(byte)ReturnVoid});
            cls.interfaces = new int[0];
            statics.static_traits.traits.clear();
            abc.findBody(statics.cinit_index).setCodeBytes(new byte[]{(byte)ReturnVoid});
            if (owner.endsWith("PetDetailSkill")) {
                cls.instance_traits.traits.removeIf(t -> !c.getString(t.getName(abc).name_index).equals("aolaUpdateSkillOnDetailBroadcast"));
                continue;
            }
            var show = body(abc, cls, "setTabInfoVisible");
            var code = show.getCode().code;
            // Keep both original nested loops, stopping before the unrelated group
            // of class-specific visibility assignments immediately following them.
            var exit = code.stream().filter(i -> i.definition.instructionCode == IfLt).toList();
            if (exit.size() != 2) throw new IllegalArgumentException("Unexpected tab traversal");
            int end = code.indexOf(exit.getLast()) + 1;
            var out = new ByteArrayOutputStream();
            for (var instruction : code.subList(0, end)) out.writeBytes(instruction.getBytes());
            out.writeBytes(op(ReturnVoid)); show.setCodeBytes(out.toByteArray());
            var event = body(abc, cls, "onCurrentPetUpdate");
            var eventCode = event.getCode().code;
            int name = trait(abc, cls, "setTabInfoVisible").name_index;
            var call = eventCode.stream().filter(i -> i.definition.instructionCode == CallPropVoid
                && i.operands[0] == name).findFirst().orElseThrow();
            int pos = eventCode.indexOf(call);
            out.reset(); out.writeBytes(op(GetLocal0)); out.writeBytes(op(PushScope));
            for (var i : eventCode.subList(pos - 1 - call.operands[1], pos + 1)) out.writeBytes(i.getBytes());
            out.writeBytes(op(ReturnVoid)); event.setCodeBytes(out.toByteArray());
            var needed = Set.of("mcskill", "mutexTabGroup0", "mutexTabGroup1Ary", "mutexInfos", "setTabInfoVisible", "onCurrentPetUpdate");
            cls.instance_traits.traits.removeIf(t -> !needed.contains(c.getString(t.getName(abc).name_index)));
            cls.instance_traits.traits.forEach(t -> t.getName(abc).namespace_index = publicNs);
        }
        abc.script_info.removeIf(s -> s.traits.traits.stream().noneMatch(
            t -> t instanceof TraitClass def && remap.containsKey(def.class_info)));
        if (abc.script_info.size() != remap.size()) throw new IllegalArgumentException("Unexpected script layout");
        for (var script : abc.script_info) {
            var def = (TraitClass)script.traits.traits.getFirst();
            int old = def.class_info;
            def.class_info = remap.get(old);
            var init = abc.findBody(script.init_index);
            var code = init.getCode();
            for (var i : code.code) if (i.definition.instructionCode == NewClass && i.operands[0] == old)
                i.operands[0] = def.class_info;
            init.setCode(code);
        }
        abc.instance_info.clear(); abc.instance_info.addAll(instances);
        abc.class_info.clear(); abc.class_info.addAll(classes);
        try (var output = Files.newOutputStream(Path.of(args[1]))) { abc.saveToStream(output); }
    }
}
