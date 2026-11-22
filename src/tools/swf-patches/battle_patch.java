import java.io.ByteArrayOutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class BattlePatch {
    static class Code extends ByteArrayOutputStream {
        void op(int opcode, int... operands) {
            writeBytes(new AVM2Instruction(0, AVM2Code.instructionSet[opcode], operands).getBytes());
        }
    }

    public static void main(String[] args) throws Exception {
        ABC abc;
        try (var input = Files.newInputStream(Path.of(args[0]))) {
            var tags = new SWF(input, false).getAbcList();
            if (tags.size() != 1) throw new IllegalArgumentException("Expected one battle ABC");
            abc = tags.getFirst().getABC();
        }
        try (var output = Files.newOutputStream(Path.of(args[1]))) { abc.saveToStream(output); }
        var c = abc.constants;
        int owner = abc.findClassByName("mmo.petfightnew.common.util.GQWanyuZhichaoEffectHelper");
        var traits = abc.class_info.get(owner).static_traits.traits;
        var methods = traits.stream().filter(t -> c.getString(t.getName(abc).name_index).equals("playAttackMovie")).toList();
        if (methods.size() != 1 || !(methods.getFirst() instanceof TraitMethodGetterSetter target)
                || target.method_info != 560 || abc.findBodyIndex(560) != 558)
            throw new IllegalArgumentException("Unexpected Wanyu helper version");
        var body = abc.bodies.get(558);
        byte[] before = body.getCodeBytes();
        if (before.length < 3 || before[0] != (byte) GetLocal0 || before[1] != (byte) PushScope ||
                before[2] != (byte) NewActivation || body.exceptions.length != 0)
            throw new IllegalArgumentException("Unexpected original method prologue or exception regions");

        // Use the nullable completion function just as the original no-effect path does.
        var invoke = new Code();
        invoke.op(GetLocal, 4);
        invoke.op(CallPropVoid, c.getPublicQnameId("apply", true), 0);
        var skip = new Code();
        skip.op(GetLocal, 4);
        skip.op(IfFalse, invoke.size());
        skip.writeBytes(invoke.toByteArray());
        skip.op(ReturnVoid);

        var updated = new Code();
        updated.write(before, 0, 2);
        updated.op(GetLex, c.getQnameId("ExternalInterface", Namespace.KIND_PACKAGE, "flash.external", true));
        updated.op(PushString, c.getStringId("aolaBlockSkill", true));
        updated.op(PushString, c.getStringId("wanyu-zhichao", true));
        updated.op(CallProperty, c.getPublicQnameId("call", true), 2);
        updated.op(PushTrue);
        updated.op(IfStrictNe, skip.size());
        updated.writeBytes(skip.toByteArray());
        updated.write(before, 2, before.length - 2);
        body.max_stack = Math.max(body.max_stack, 3);
        body.setCodeBytes(updated.toByteArray());
        try (var output = Files.newOutputStream(Path.of(args[2]))) { abc.saveToStream(output); }
        System.out.println("GQWanyuZhichaoEffectHelper.playAttackMovie: method 560 body 558; original bytes retained; inserted " +
                (updated.size() - before.length) + " bytes");
    }
}
