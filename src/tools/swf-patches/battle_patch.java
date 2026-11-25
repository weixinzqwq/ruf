import java.io.ByteArrayOutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class BattlePatch {
    static class Code extends ByteArrayOutputStream {
        void op(int opcode, int... operands) {
            writeBytes(instruction(opcode, operands).getBytes());
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
        patchWanyu(abc);
        patchPortrait(abc, "showJuexingAppearMovie", 1757, 0x85, "juexing",
                "gqpet/juexing/effect/juexingeffect_");
        patchPortrait(abc, "showJiuRiShenAppearMovie", 1762, 0x82, "jiuri",
                "gqpet/jiurichongxian/effect/jiurichongxianeffect_");
        patchPortrait(abc, "showDengShenChangJieAppearMovie", 1767, 0x85, "dengshen",
                "gqpet/dengshenchangjie/effect/dengshenchangjieeffect_");
        patchPortrait(abc, "showZhenLiFaDianAppearMovie", 1772, 0x82, "zhenli",
                "gqpet/zhenlifadian/effect/zhenlifadianeffect_");
        patchPortrait(abc, "showYiRenMovie", 1814, 0x112, "yiren",
                "gqpet/yiren/effect/yirenfirstappeareffect");
        patchBackdrop(abc, "StartMovie", 5299, 0x5f, 0x27, 0x49);
        patchBackdrop(abc, "StartMovie3v3", 1253, 0x3d, 0x27);
        try (var output = Files.newOutputStream(Path.of(args[2]))) { abc.saveToStream(output); }
    }

    static void patchWanyu(ABC abc) {
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
        System.out.println("GQWanyuZhichaoEffectHelper.playAttackMovie: method 560 body 558; original bytes retained; inserted " +
                (updated.size() - before.length) + " bytes");
    }

    static void expectBytes(byte[] code, int offset, byte[] expected) {
        if (!Arrays.equals(code, offset, offset + expected.length, expected, 0, expected.length)) {
            throw new IllegalArgumentException("Unexpected instructions at " + offset);
        }
    }

    static void patchBackdrop(ABC abc, String ownerName, int method, int completionAt, int... requests) throws Exception {
        var c = abc.constants;
        int owner = abc.findClassByName("mmo.petfightnew.common.startmovie." + ownerName);
        var traits = abc.instance_info.get(owner).instance_traits.traits;
        var targets = traits.stream().filter(t -> c.getString(t.getName(abc).name_index).equals("enterFight")).toList();
        if (targets.size() != 1 || !(targets.getFirst() instanceof TraitMethodGetterSetter target)
                || target.method_info != method || abc.findBodyIndex(method) != method - 2)
            throw new IllegalArgumentException("Unexpected battle entry method in " + ownerName);
        var body = abc.findBody(method);
        byte[] before = body.getCodeBytes();
        var completion = new Code();
        completion.op(GetScopeObject, 1);
        completion.op(GetSlot, 1);
        completion.op(GetGlobalScope);
        completion.op(Call, 0);
        completion.op(Pop);
        completion.op(ReturnVoid);
        expectBytes(before, completionAt, completion.toByteArray());
        if (before.length != completionAt + completion.size() || body.exceptions.length != 0)
            throw new IllegalArgumentException("Unexpected completion layout in " + ownerName);

        var code = body.getCode();
        var done = code.code.get(code.adr2pos(completionAt));
        var jumps = new ArrayList<AVM2Instruction>();
        // Work backwards so the earlier request still has its original address.
        for (int i = requests.length - 1; i >= 0; i--) {
            int at = requests[i];
            boolean gq = requests.length == 2 && i == 0;
            String asset = gq ? "mmo.petfight.GQcommonBackMv" : "mmo.petfight.XinghuicommonBackMv";
            var request = new Code();
            request.op(GetLex, c.getQnameId("ShowMovieOrCartoonHelper", Namespace.KIND_PACKAGE, "mmo.common.simplemoviehelper", false));
            request.op(PushTrue);
            request.op(PushString, c.getStringId("petfightbackground/common", false));
            request.op(PushString, c.getStringId(asset, false));
            expectBytes(before, at, request.toByteArray());

            int pos = code.adr2pos(at);
            var jump = instruction(IfStrictEq, 0);
            // Only guard a request that the original pet flags selected. The
            // fallback target is the existing onEnterFight() call after dispose.
            for (var op : new AVM2Instruction[]{
                    instruction(GetLex, c.getQnameId("ExternalInterface", Namespace.KIND_PACKAGE, "flash.external", true)),
                    instruction(PushString, c.getStringId("aolaBlockSkill", true)),
                    instruction(PushString, c.getStringId(gq ? "battle-backdrop-gq" : "battle-backdrop-xinghui", true)),
                    instruction(CallProperty, c.getPublicQnameId("call", true), 2), instruction(PushTrue), jump}) {
                code.insertInstruction(pos++, op, body);
            }
            jumps.add(jump);
        }
        for (var jump : jumps) jump.operands[0] = (int)(done.getAddress() - jump.getAddress() - jump.getBytesLength());
        code.checkValidOffsets(body);
        body.setCode(code);
        System.out.printf("%s.enterFight: method %d, %d background request guards%n", ownerName, method, requests.length);
    }

    static AVM2Instruction instruction(int opcode, int... operands) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[opcode], operands);
    }

    static void patchPortrait(ABC abc, String name, int method, int loadAt, String channel, String asset) {
        var c = abc.constants;
        int owner = abc.findClassByName("mmo.petfightnew.common.fightcontroller.ChangePetView");
        var traits = abc.instance_info.get(owner).instance_traits.traits;
        var targets = traits.stream().filter(t -> c.getString(t.getName(abc).name_index).equals(name)).toList();
        if (targets.size() != 1 || !(targets.getFirst() instanceof TraitMethodGetterSetter target)
                || target.method_info != method || abc.findBodyIndex(method) != method - 2)
            throw new IllegalArgumentException("Unexpected transform method: " + name);
        MethodBody body = abc.findBody(method);
        byte[] before = body.getCodeBytes();
        var loader = new Code();
        loader.op(GetLex, c.getQnameId("ResScheduler", Namespace.KIND_PACKAGE, "mmo.loader.scheduler", false));
        loader.op(GetLex, c.getQnameId("ResParams", Namespace.KIND_PACKAGE, "mmo.loader.scheduler", false));
        loader.op(PushString, c.getStringId(asset, false));
        expectBytes(before, loadAt, loader.toByteArray());

        // The error handler in the original request is a closure inside a with
        // scope. Recreate the scope and call that very function at the load site.
        int fallback = method - 1;
        var closure = new Code();
        closure.op(NewObject, 0);
        closure.op(PushWith);
        closure.op(NewFunction, fallback);
        var registration = new Code();
        registration.writeBytes(closure.toByteArray());
        registration.op(Dup);
        registration.op(GetScopeObject, 2);
        registration.op(Swap);
        registration.op(SetProperty, c.getPublicQnameId("onIOError", false));
        registration.op(PopScope);
        expectBytes(before, loadAt + (method == 1814 ? 0x2d : 0x36), registration.toByteArray());
        // The single reference parameter is unused by these handlers.
        int errorEvent = c.getQnameId("IOErrorEvent", Namespace.KIND_PACKAGE, "flash.events", false);
        if (body.exceptions.length != 0 || !Arrays.equals(abc.method_info.get(fallback).param_types, new int[]{errorEvent}))
            throw new IllegalArgumentException("Unexpected error handler contract for " + name);

        var skip = new Code();
        if (method == 1814) {
            // Preserve the first-appearance marker before the callback can start
            // another transform. The repeated local effect bypasses this guard.
            if (before.length != 0x166 || before[0x165] != (byte) ReturnVoid)
                throw new IllegalArgumentException("Unexpected first-appearance marker layout");
            skip.write(before, 0x155, 0x165 - 0x155);
        }
        // The game's replacement removes the old child, then inserts at index+1.
        // When the old child was topmost that index would exceed numChildren.
        // Keep the stock insertion order whenever its target is still available.
        int indexSlot = method == 1814 ? 9 : 6;
        var topmost = new Code();
        topmost.op(GetScopeObject, 1);
        topmost.op(Dup);
        topmost.op(GetSlot, indexSlot);
        topmost.op(DecrementI);
        topmost.op(SetSlot, indexSlot);
        skip.op(GetScopeObject, 1);
        skip.op(GetSlot, indexSlot);
        skip.op(IncrementI);
        skip.op(GetScopeObject, 1);
        skip.op(GetSlot, 5); // curMCPetContainer
        skip.op(GetProperty, c.getPublicQnameId("parent", false));
        skip.op(GetProperty, c.getPublicQnameId("numChildren", false));
        skip.op(IfLt, topmost.size());
        skip.writeBytes(topmost.toByteArray());
        skip.writeBytes(closure.toByteArray());
        skip.op(PopScope);
        skip.op(PushNull); // receiver
        skip.op(PushNull); // unused error argument
        skip.op(Call, 1);
        skip.op(Pop);
        skip.op(ReturnVoid);

        var guard = new Code();
        guard.op(GetLex, c.getQnameId("ExternalInterface", Namespace.KIND_PACKAGE, "flash.external", true));
        guard.op(PushString, c.getStringId("aolaBlockSkill", true));
        guard.op(PushString, c.getStringId("transform-portrait-" + channel, true));
        guard.op(CallProperty, c.getPublicQnameId("call", true), 2);
        guard.op(PushTrue);
        guard.op(IfStrictNe, skip.size());
        guard.writeBytes(skip.toByteArray());

        if (method == 1814) {
            // The condition's destination is the start of our guard. Only the
            // unconditional jump at the end of the repeat branch crosses it.
            var jump = new Code();
            jump.op(Jump, 0x165 - loadAt);
            expectBytes(before, loadAt - 4, jump.toByteArray());
            jump.reset();
            jump.op(Jump, 0x165 - loadAt + guard.size());
            System.arraycopy(jump.toByteArray(), 0, before, loadAt - 4, 4);
        }
        var updated = new Code();
        updated.write(before, 0, loadAt);
        updated.writeBytes(guard.toByteArray());
        updated.write(before, loadAt, before.length - loadAt);
        body.setCodeBytes(updated.toByteArray());
        System.out.printf("ChangePetView.%s: method %d, body %d, guarded request at 0x%x, fallback %d%n",
                name, method, method - 2, loadAt, fallback);
    }
}
