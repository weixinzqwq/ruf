import java.util.ArrayList;
import java.util.List;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

public class PetListSwitchPatch {
    public static void apply(ABC abc) throws Exception {
        int owner = abc.findClassByName("mmo.petpackage.panel.inclusion.petlist.PetSingleIconBase");
        for (String name : List.of("updateUI", "nullPet")) {
            var methods = abc.instance_info.get(owner).instance_traits.traits.stream()
                .filter(t -> abc.constants.getString(t.getName(abc).name_index).equals(name)).toList();
            if (methods.size() != 1 || !(methods.getFirst() instanceof TraitMethodGetterSetter method))
                throw new IllegalArgumentException("Expected PetSingleIconBase." + name);
            guardStatusFrames(abc, abc.findBody(method.method_info), name.equals("nullPet"));
        }
    }

    private static AVM2Instruction op(int opcode, int... operands) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[opcode], operands);
    }

    private static void guardStatusFrames(ABC abc, MethodBody body, boolean empty) throws Exception {
        var c = abc.constants;
        var code = body.getCode();
        int go = c.getPublicQnameId("gotoAndStop", false);
        var calls = code.code.stream().filter(i -> i.definition.instructionCode == CallPropVoid
                && i.operands[0] == go && i.operands[1] == 1).toList();
        record Site(String clip, int receiverDistance, int frameOpcode, int frameOperand) {}
        var expected = empty ? List.of(new Site("mcNormal", 2, PushByte, 1)) : List.of(
            new Site("mcHp", 14, CallProperty, c.getPublicQnameId("int", false)),
            new Site("mcSelect", 3, GetSlot, 5), new Site("mcNormal", 3, GetSlot, 5));
        if (calls.size() != expected.size()) throw new IllegalArgumentException("Unexpected number of slot gotos");
        // Verify the concrete clip at each call. The HP frame is the original
        // int(totalFrames * curHp / maxHp); category frames live in activation slot 5.
        for (int i = 0; i < calls.size(); i++) {
            int pos = code.code.indexOf(calls.get(i));
            var site = expected.get(i);
            var receiver = code.code.get(pos - site.receiverDistance());
            if (receiver.definition.instructionCode != GetProperty
                    || !c.getString(c.getMultiname(receiver.operands[0]).name_index).equals(site.clip()))
                throw new IllegalArgumentException("Unrecognized status clip at " + calls.get(i).getAddress());
            var arg = code.code.get(pos - 1);
            if (arg.definition.instructionCode != site.frameOpcode() || arg.operands[0] != site.frameOperand())
                throw new IllegalArgumentException("Unrecognized status frame expression");
        }
        var branches = new ArrayList<AVM2Instruction[]>();
        int frame = body.max_regs++;
        body.max_stack++;
        for (var call : calls) {
            int pos = code.code.indexOf(call);
            var next = code.code.get(pos + 1);
            var changed = op(IfStrictNe, 0);
            var finished = op(Jump, 0);
            var restoreFrame = op(GetLocal, frame);
            // At this point the caller has already evaluated both the MovieClip
            // and its integer target. A same-frame stop must still halt playback.
            for (var ins : List.of(op(SetLocal, frame), op(Dup), op(GetProperty, c.getPublicQnameId("currentFrame", true)),
                    op(GetLocal, frame), changed, op(CallPropVoid, c.getPublicQnameId("stop", true), 0),
                    finished, restoreFrame)) {
                code.insertInstruction(pos++, ins, body);
            }
            branches.add(new AVM2Instruction[]{changed, restoreFrame});
            branches.add(new AVM2Instruction[]{finished, next});
        }
        for (var pair : branches)
            pair[0].operands[0] = (int)(pair[1].getAddress() - pair[0].getAddress() - pair[0].getBytesLength());
        code.checkValidOffsets(body);
        body.setCode(code);
    }
}
