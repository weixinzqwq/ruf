import java.io.ByteArrayOutputStream;
import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

// Lift the serialized production guard and its entire true branch. Only ABC-local
// constant-pool indices (public fixture fields stand in for private fields) and
// branch distances change. False falls through to the fixture's original body.
class MyProEffectsProbeSites {
    static void lift(ABC game, ABC probe, String owner, String member, String flag, String target) throws Exception {
        var body = method(game, "mmo.petfightnew.common." + owner, member);
        var code = body.getCode();
        int helper = game.constants.getQnameId("MyProBattleEffects", Namespace.KIND_PACKAGE, "aola.tools", false);
        var start = code.code.stream().filter(i -> i.definition.instructionCode == GetLex && i.operands[0] == helper)
                .findFirst().orElseThrow();
        int first = code.code.indexOf(start);
        int branch = first;
        while (code.code.get(branch).definition.instructionCode != IfFalse) branch++;
        var call = code.code.get(branch - 1);
        if (call.definition.instructionCode != CallProperty || call.operands[0] != game.constants.getPublicQnameId(flag, false))
            throw new IllegalArgumentException("Unexpected switch for " + target);
        int end = code.adr2pos(code.code.get(branch).getTargetAddress());
        var added = new ArrayList<AVM2Instruction>();
        var offsets = new HashMap<Long,Integer>();
        int length = 0;
        for (var i : code.code.subList(first, end)) {
            int opcode = i.definition.instructionCode;
            int[] args = i.operands == null ? null : i.operands.clone();
            if (opcode == GetLex || opcode == GetProperty || opcode == CallProperty || opcode == CallPropVoid) {
                var mn = game.constants.getMultiname(args[0]);
                String name = game.constants.getString(mn.name_index);
                args[0] = opcode == GetLex
                        ? probe.constants.getQnameId(name, Namespace.KIND_PACKAGE,
                                mn.getNamespace(game.constants).getName(game.constants).toRawString(), true)
                        : probe.constants.getPublicQnameId(name, true);
            }
            var mapped = new AVM2Instruction(0, i.definition, args);
            offsets.put(i.getAddress(), length);
            length += mapped.getBytesLength();
            added.add(mapped);
        }
        offsets.put(code.code.get(end).getAddress(), length);
        var bytes = new ByteArrayOutputStream();
        for (int k = 0; k < added.size(); k++) {
            var mapped = added.get(k);
            if (mapped.isBranch()) mapped.operands[0] = offsets.get(code.code.get(first + k).getTargetAddress())
                    - (bytes.size() + mapped.getBytesLength());
            bytes.writeBytes(mapped.getBytes());
        }
        var cls = probe.instance_info.get(probe.findClassByName("probe.GameEffects"));
        var method = (TraitMethodGetterSetter)cls.instance_traits.traits.stream().filter(t ->
                target.equals(probe.constants.getString(t.getName(probe).name_index))).findFirst().orElseThrow();
        var fixture = probe.findBody(method.method_info);
        // ASC emits no pushscope for some trivial methods. The guard uses getlex.
        var out = new ByteArrayOutputStream();
        out.write(GetLocal0); out.write(PushScope); out.writeBytes(bytes.toByteArray());
        out.write(PopScope); out.writeBytes(fixture.getCodeBytes());
        fixture.max_stack = Math.max(fixture.max_stack, 5);
        fixture.max_scope_depth++;
        fixture.setCodeBytes(out.toByteArray());
        fixture.getCode().checkValidOffsets(fixture);
    }

    static MethodBody method(ABC abc, String owner, String target) {
        var cls = abc.instance_info.get(abc.findClassByName(owner));
        var method = (TraitMethodGetterSetter)cls.instance_traits.traits.stream().filter(t ->
                target.equals(abc.constants.getString(t.getName(abc).name_index))).findFirst().orElseThrow();
        return abc.findBody(method.method_info);
    }

    public static void main(String[] args) throws Exception {
        try (var input = Files.newInputStream(Path.of(args[0])); var mock = Files.newInputStream(Path.of(args[1]));
             var output = Files.newOutputStream(Path.of(args[2]))) {
            var game = new SWF(input, false).getAbcList().getLast().getABC();
            var probe = new SWF(mock, false).getAbcList().getFirst().getABC();
            lift(game, probe, "SkillResultMovieHandler", "showReallyHit", "damage", "hit");
            lift(game, probe, "SkillResultMovieHandler", "showIncMovie", "healing", "heal");
            lift(game, probe, "SkillResultMovieHandler", "showMultipleHit", "multiple", "multi");
            lift(game, probe, "fightcontroller.SwitchPm", "showSwitchMc", "skip", "change");
            lift(game, probe, "fightcontroller.UseSkill", "handleMovie", "skip", "skill");
            lift(game, probe, "fightcontroller.BattleNextTurn", "checkPlayMovie", "skip", "next");
            lift(game, probe, "fightcontroller.UseSkill", "getMuiltHitDecHpDelayFrame", "skip", "delay");
            probe.saveToStream(output);
        }
    }
}
