import java.io.ByteArrayOutputStream;
import java.nio.file.Files;
import java.nio.file.Path;

import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.InstanceInfo;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.Trait;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

// Patch only the six display methods in the archived messagesendservice. Relative
// branches in the original code retain their bytes and their destinations.
class NoticePatch {
    static class Code extends ByteArrayOutputStream {
        void op(int opcode, int... operands) {
            writeBytes(new AVM2Instruction(0, AVM2Code.instructionSet[opcode], operands).getBytes());
        }
    }

    static Trait trait(ABC abc, InstanceInfo owner, String name) {
        var matches = owner.instance_traits.traits.stream()
            .filter(t -> abc.constants.getString(t.getName(abc).name_index).equals(name)).toList();
        if (matches.size() != 1) throw new IllegalArgumentException("Missing or ambiguous trait: " + name);
        return matches.getFirst();
    }

    static void patch(ABC abc, String ownerName, String methodName, int expectedBody, String channel) {
        var constants = abc.constants;
        int ownerId = abc.findClassByName("mmo.MessageSend." + ownerName);
        if (ownerId < 0) throw new IllegalArgumentException("Missing class: " + ownerName);
        var owner = abc.instance_info.get(ownerId);
        var method = (TraitMethodGetterSetter) trait(abc, owner, methodName);
        if (abc.findBodyIndex(method.method_info) != expectedBody)
            throw new IllegalArgumentException("Unexpected body for " + methodName);
        MethodBody body = abc.bodies.get(expectedBody);
        byte[] before = body.getCodeBytes();
        if (before.length < 3 || before[0] != (byte) GetLocal0 || before[1] != (byte) PushScope ||
            body.exceptions.length != 0)
            throw new IllegalArgumentException("Unsupported prologue or exception region: " + methodName);
        // Both targets of every newly inserted conditional branch have an empty
        // operand stack. The original prologue establishes the scope before getlex.
        var skip = new Code();
        int length = constants.getPublicQnameId("length", false);
        int queue = 0;
        if (methodName.equals("messageComing")) {
            queue = trait(abc, owner, "messages").name_index;
            skip.op(GetLocal0);
            skip.op(GetProperty, queue);
            skip.op(PushByte, 0);
            skip.op(SetProperty, length);
        } else if (methodName.equals("onSystemMsgComes")) {
            skip.op(GetLocal0);
            skip.op(GetProperty, trait(abc, owner, "sTimer").name_index);
            skip.op(CallPropVoid, constants.getPublicQnameId("start", false), 0);
        }
        skip.op(ReturnVoid);
        var guard = new Code();
        guard.op(GetLex, constants.getQnameId("ExternalInterface", Namespace.KIND_PACKAGE, "flash.external", true));
        guard.op(PushString, constants.getStringId("aolaHideNotice", true));
        guard.op(PushString, constants.getStringId(channel, true));
        guard.op(CallProperty, constants.getPublicQnameId("call", true), 2);
        guard.op(PushTrue);
        guard.op(IfStrictNe, skip.size());
        guard.writeBytes(skip.toByteArray());
        var prefix = new Code();
        if (methodName.equals("messageComing")) {
            // A frame with an empty display queue never calls the host.
            prefix.op(GetLocal0);
            prefix.op(GetProperty, queue);
            prefix.op(GetProperty, length);
            prefix.op(IfFalse, guard.size());
        }
        prefix.writeBytes(guard.toByteArray());
        var result = new Code();
        result.write(before, 0, 2);
        result.writeBytes(prefix.toByteArray());
        result.write(before, 2, before.length - 2);
        body.max_stack = Math.max(body.max_stack, 3);
        body.setCodeBytes(result.toByteArray());
        System.out.println(ownerName + "." + methodName + " body=" + expectedBody +
                           " channel=" + channel + " inserted=" + prefix.size());
    }

    static void save(ABC abc, String file) throws Exception {
        try (var output = Files.newOutputStream(Path.of(file))) {
            abc.saveToStream(output);
        }
    }

    public static void main(String[] args) throws Exception {
        ABC abc;
        try (var input = Files.newInputStream(Path.of(args[0]))) {
            var tags = new SWF(input, false).getAbcList();
            if (tags.size() != 1) throw new IllegalArgumentException("Expected one DoABC");
            abc = tags.getFirst().getABC();
        }
        if (abc.constants.getStringId("aolaHideNotice", false) > 0)
            throw new IllegalArgumentException("Input already contains a notice patch");
        save(abc, args[1]);
        patch(abc, "MessageRandomShow", "initSystemdMsg", 13, "random");
        patch(abc, "MessageRandomShow", "onSystemMsgComes", 15, "random");
        patch(abc, "MessageSendService", "onHonorMsgDispatch", 63, "honor");
        patch(abc, "MessageSendService", "onTitleHandle", 64, "title");
        patch(abc, "MessageSendService", "onReturnBroadcast", 65, "broadcast");
        patch(abc, "MessageSendService", "messageComing", 71, "coming");
        save(abc, args[2]);
    }
}
