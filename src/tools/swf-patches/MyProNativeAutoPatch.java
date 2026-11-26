import java.nio.file.Files;
import java.nio.file.Path;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class MyProNativeAutoPatch {
    static int callSite(MethodBody body, int name, int listener) {
        var code = body.getCode().code;
        int at = -1;
        for (int i = 0; i < code.size(); i++) {
            var instruction = code.get(i);
            if (instruction.definition.instructionCode == CallPropVoid && instruction.operands[0] == name) {
                if (listener != 0 && (i == 0 || code.get(i - 1).definition.instructionCode != GetProperty ||
                    code.get(i - 1).operands[0] != listener)) continue;
                if (at >= 0) throw new IllegalArgumentException("Ambiguous button call");
                at = i;
            }
        }
        if (at < 0) throw new IllegalArgumentException("Native button call missing");
        return at;
    }

    static final String PANEL = "mmo.petfightnew.common.panel.skill.NewSkillPanel";

    static ABC game(String file) throws Exception {
        try (var stream = Files.newInputStream(Path.of(file))) {
            var candidates = new SWF(stream, false).getAbcList().stream().map(tag -> tag.getABC())
                .filter(abc -> abc.findClassByName(PANEL) >= 0).toList();
            if (candidates.size() != 1) throw new IllegalArgumentException("Expected one native skill panel");
            return candidates.getFirst();
        }
    }

    public static void main(String[] args) throws Exception {
        ABC abc = game(args[0]);
        ABC verify = args.length == 2 ? game(args[1]) : null;
        if (verify == null) {
            try (var out = Files.newOutputStream(Path.of(args[1]))) { abc.saveToStream(out); }
        }
        var c = abc.constants;
        var traits = abc.instance_info.get(abc.findClassByName(PANEL)).instance_traits.traits;
        java.util.function.Function<String, Integer> member = name -> {
            var found = traits.stream().filter(t -> name.equals(c.getString(t.getName(abc).name_index))).toList();
            if (found.size() != 1) throw new IllegalArgumentException("Missing or ambiguous " + name);
            return found.getFirst().name_index;
        };
        java.util.function.Function<String, MethodBody> method = name -> {
            int qname = member.apply(name);
            var trait = (TraitMethodGetterSetter) traits.stream().filter(t -> t.name_index == qname).findFirst().orElseThrow();
            return abc.findBody(trait.method_info);
        };
        int controller = c.getQnameId("FightController", Namespace.KIND_PACKAGE, "mmo.petfightnew.common", false);
        int data = c.getQnameId("FightData", Namespace.KIND_PACKAGE, "mmo.petfightnew.common", false);
        if (controller <= 0 || data <= 0) throw new IllegalArgumentException("Missing native game classes");
        int button = member.apply("mcAutoUseSkill");
        callSite(method.apply("changeBtnState"), c.getPublicQnameId("autoUseAI", false), 0);
        int bridge = c.getQnameId("MyProNativeAuto", Namespace.KIND_PACKAGE, "aola.tools", true);
        var sites = new java.util.HashSet<Integer>();
        for (boolean bind : new boolean[]{true, false}) {
            var body = method.apply(bind ? "initAutoUseSkillMC" : "dispose");
            int call = callSite(body, c.getPublicQnameId(bind ? "addEventListener" : "removeEventListener", false),
                member.apply("changeBtnState"));
            // dispose removes several listeners. The button's bound changeBtnState method identifies its call.
            int action = c.getPublicQnameId(bind ? "bind" : "unbind", true);
            AVM2Instruction[] added = bind ? new AVM2Instruction[]{
                    MyProBattlePatch.op(GetLex, bridge),
                    MyProBattlePatch.op(GetLocal0), MyProBattlePatch.op(GetProperty, button),
                    MyProBattlePatch.op(GetLex, controller), MyProBattlePatch.op(GetLex, data),
                    MyProBattlePatch.op(CallPropVoid, action, 3)} : new AVM2Instruction[]{
                    MyProBattlePatch.op(GetLex, bridge),
                    MyProBattlePatch.op(GetLocal0), MyProBattlePatch.op(GetProperty, button),
                    MyProBattlePatch.op(CallPropVoid, action, 1)};
            if (verify == null) {
                MyProBattlePatch.insert(body, call + 1, added);
                body.max_stack = Math.max(body.max_stack, bind ? 4 : 2);
            } else {
                MyProBattlePatch.validateSite(body, verify.findBody(body.method_info), call + 1, added);
                sites.add(body.method_info);
            }
        }
        if (verify != null) {
            if (abc.bodies.size() != verify.bodies.size()) throw new IllegalArgumentException("Method count changed");
            for (var body : abc.bodies) {
                if (!sites.contains(body.method_info) && !java.util.Arrays.equals(body.getCodeBytes(),
                        verify.findBody(body.method_info).getCodeBytes()))
                    throw new IllegalArgumentException("Unexpected change to native method " + body.method_info);
            }
            return;
        }
        try (var out = Files.newOutputStream(Path.of(args[2]))) { abc.saveToStream(out); }
    }
}
