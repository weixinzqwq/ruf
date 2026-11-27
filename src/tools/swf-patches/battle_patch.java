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
        patchPortrait(abc, "showJuexingAppearMovie", "juexing",
                "gqpet/juexing/effect/juexingeffect_");
        patchPortrait(abc, "showJiuRiShenAppearMovie", "jiuri",
                "gqpet/jiurichongxian/effect/jiurichongxianeffect_");
        patchPortrait(abc, "showDengShenChangJieAppearMovie", "dengshen",
                "gqpet/dengshenchangjie/effect/dengshenchangjieeffect_");
        patchPortrait(abc, "showZhenLiFaDianAppearMovie", "zhenli",
                "gqpet/zhenlifadian/effect/zhenlifadianeffect_");
        patchPortrait(abc, "showYiRenMovie", "yiren",
                "gqpet/yiren/effect/yirenfirstappeareffect");
        patchBackdrop(abc, "StartMovie", true);
        patchBackdrop(abc, "StartMovie3v3", false);
        try (var output = Files.newOutputStream(Path.of(args[2]))) { abc.saveToStream(output); }
    }

    static MethodBody namedMethod(ABC abc, String owner, String name, boolean isStatic) {
        int id = abc.findClassByName(owner);
        if (id < 0) throw new IllegalArgumentException("Missing class " + owner + " for " + name);
        var traits = (isStatic ? abc.class_info.get(id).static_traits : abc.instance_info.get(id).instance_traits).traits;
        var matches = traits.stream().filter(t -> t instanceof TraitMethodGetterSetter
                && name.equals(abc.constants.getString(t.getName(abc).name_index))).toList();
        if (matches.size() != 1) throw new IllegalArgumentException("Missing or ambiguous method " + owner + "." + name);
        var body = abc.findBody(((TraitMethodGetterSetter) matches.getFirst()).method_info);
        if (body == null) throw new IllegalArgumentException("Missing method body " + owner + "." + name);
        return body;
    }

    static void patchWanyu(ABC abc) {
        var c = abc.constants;
        String owner = "mmo.petfightnew.common.util.GQWanyuZhichaoEffectHelper";
        var body = namedMethod(abc, owner, "playAttackMovie", true);
        var params = abc.method_info.get(body.method_info).param_types;
        byte[] before = body.getCodeBytes();
        if (before.length < 3 || before[0] != (byte) GetLocal0 || before[1] != (byte) PushScope ||
                before[2] != (byte) NewActivation || body.exceptions.length != 0 || params.length < 4 || params[3] == 0
                || !"Function".equals(c.getString(c.getMultiname(params[3]).name_index)))
            throw new IllegalArgumentException(owner + ".playAttackMovie expected GetLocal0/PushScope/NewActivation, no exception regions and Function argument 4");

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
        System.out.println("GQWanyuZhichaoEffectHelper.playAttackMovie: method " + body.method_info + "; original bytes retained; inserted " +
                (updated.size() - before.length) + " bytes");
    }

    static boolean matchesBytes(byte[] code, int offset, byte[] expected) {
        return offset >= 0 && offset + expected.length <= code.length &&
                Arrays.equals(code, offset, offset + expected.length, expected, 0, expected.length);
    }

    static int findAnchor(MethodBody body, byte[] original, Code pattern, String label) {
        byte[] expected = pattern.toByteArray();
        int result = -1;
        for (var ins : body.getCode().code) {
            int offset = (int) ins.getAddress();
            if (matchesBytes(original, offset, expected)) {
                if (result >= 0) throw new IllegalArgumentException("Ambiguous anchor " + label);
                result = offset;
            }
        }
        return result;
    }

    static int anchor(MethodBody body, byte[] original, Code pattern, String label) {
        int offset = findAnchor(body, original, pattern, label);
        if (offset < 0) throw new IllegalArgumentException("Missing anchor " + label);
        return offset;
    }

    static void patchBackdrop(ABC abc, String ownerName, boolean includesGq) throws Exception {
        var c = abc.constants;
        String owner = "mmo.petfightnew.common.startmovie." + ownerName;
        var body = namedMethod(abc, owner, "enterFight", false);
        byte[] before = body.getCodeBytes();
        var completion = new Code();
        completion.op(GetScopeObject, 1);
        completion.op(GetSlot, 1);
        completion.op(GetGlobalScope);
        completion.op(Call, 0);
        completion.op(Pop);
        completion.op(ReturnVoid);
        int completionAt = anchor(body, before, completion, owner + ".enterFight completion callback");
        if (before.length != completionAt + completion.size() || body.exceptions.length != 0)
            throw new IllegalArgumentException(owner + ".enterFight expected terminal completion callback and no exception regions");

        String[] assets = includesGq ? new String[]{"GQcommonBackMv", "XinghuicommonBackMv"}
                : new String[]{"XinghuicommonBackMv"};
        int[] requests = new int[assets.length];
        for (int i = 0; i < assets.length; i++) {
            var request = new Code();
            request.op(GetLex, c.getQnameId("ShowMovieOrCartoonHelper", Namespace.KIND_PACKAGE, "mmo.common.simplemoviehelper", false));
            request.op(PushTrue);
            request.op(PushString, c.getStringId("petfightbackground/common", false));
            request.op(PushString, c.getStringId("mmo.petfight." + assets[i], false));
            requests[i] = anchor(body, before, request, owner + ".enterFight ShowMovieOrCartoonHelper " + assets[i]);
            if (requests[i] >= completionAt || i > 0 && requests[i] <= requests[i - 1])
                throw new IllegalArgumentException(owner + ".enterFight unexpected background request order");
        }

        var code = body.getCode();
        var done = code.code.get(code.adr2pos(completionAt));
        var jumps = new ArrayList<AVM2Instruction>();
        // Work backwards so the earlier request still has its original address.
        for (int i = requests.length - 1; i >= 0; i--) {
            int at = requests[i];
            boolean gq = requests.length == 2 && i == 0;
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
        System.out.printf("%s.enterFight: method %d, %d background request guards%n", ownerName, body.method_info, requests.length);
    }

    static AVM2Instruction instruction(int opcode, int... operands) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[opcode], operands);
    }

    static void patchPortrait(ABC abc, String name, String channel, String asset) throws Exception {
        var c = abc.constants;
        String owner = "mmo.petfightnew.common.fightcontroller.ChangePetView";
        String label = owner + "." + name;
        MethodBody body = namedMethod(abc, owner, name, false);
        var code = body.getCode();
        boolean firstYiren = channel.equals("yiren");
        byte[] before = body.getCodeBytes();
        var loader = new Code();
        loader.op(GetLex, c.getQnameId("ResScheduler", Namespace.KIND_PACKAGE, "mmo.loader.scheduler", false));
        loader.op(GetLex, c.getQnameId("ResParams", Namespace.KIND_PACKAGE, "mmo.loader.scheduler", false));
        loader.op(PushString, c.getStringId(asset, false));
        int loadAt = anchor(body, before, loader, label + " ResScheduler/ResParams " + asset);

        // The error handler in the original request is a closure inside a with
        // scope. Recreate the scope and call that very function at the load site.
        int fallback = -1;
        int errorKey = c.getPublicQnameId("onIOError", false);
        int loadIndex = code.adr2pos(loadAt);
        for (int i = loadIndex + 2; i < code.code.size(); i++) {
            var ins = code.code.get(i);
            if (ins.definition.instructionCode != NewFunction) continue;
            var registered = new Code();
            registered.op(NewObject, 0);
            registered.op(PushWith);
            registered.op(NewFunction, ins.operands[0]);
            registered.op(Dup);
            registered.op(GetScopeObject, 2);
            registered.op(Swap);
            registered.op(SetProperty, errorKey);
            registered.op(PopScope);
            if (matchesBytes(before, (int) code.code.get(i - 2).getAddress(), registered.toByteArray())) {
                if (fallback >= 0) throw new IllegalArgumentException(label + " ambiguous onIOError closure");
                fallback = ins.operands[0];
            }
        }
        int errorEvent = c.getQnameId("IOErrorEvent", Namespace.KIND_PACKAGE, "flash.events", false);
        if (fallback < 0 || body.exceptions.length != 0 ||
                !Arrays.equals(abc.method_info.get(fallback).param_types, new int[]{errorEvent}))
            throw new IllegalArgumentException(label + " expected one onIOError closure with IOErrorEvent parameter");

        var closure = new Code();
        closure.op(NewObject, 0);
        closure.op(PushWith);
        closure.op(NewFunction, fallback);

        var skip = new Code();
        AVM2Instruction repeatJump = null;
        if (firstYiren) {
            // Preserve the first-appearance marker before the callback can start
            // another transform. The repeated local effect bypasses this guard.
            var marker = new Code();
            marker.op(GetLex, c.getQnameId("FightData", Namespace.KIND_PACKAGE, "mmo.petfightnew.common", false));
            marker.op(GetProperty, c.getPublicQnameId("instance", false));
            marker.op(GetProperty, c.getPublicQnameId("extData", false));
            marker.op(GetScopeObject, 1);
            marker.op(GetSlot, 8);
            marker.op(PushTrue);
            int markerAt = anchor(body, before, marker, label + " FightData.extData first-appearance marker");
            int markerEnd = code.adr2pos(markerAt + marker.size());
            var ret = code.code.getLast();
            repeatJump = code.code.get(loadIndex - 1);
            if (markerAt <= loadAt || markerEnd != code.code.size() - 2 ||
                    code.code.get(markerEnd).definition.instructionCode != SetProperty ||
                    ret.definition.instructionCode != ReturnVoid ||
                    repeatJump.definition.instructionCode != Jump || repeatJump.getTargetAddress() != ret.getAddress())
                throw new IllegalArgumentException(label + " expected first-appearance marker and repeat jump to return");
            skip.write(before, markerAt, (int) ret.getAddress() - markerAt);
        }
        // The game's replacement removes the old child, then inserts at index+1.
        // When the old child was topmost that index would exceed numChildren.
        // Keep the stock insertion order whenever its target is still available.
        int indexSlot = firstYiren ? 9 : 6;
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

        // Keep condition targets on the new guard. The original repeat branch
        // jumps over the guard and the first-appearance path to the return.
        for (var ins : code.code) {
            if (ins.definition.instructionCode == LookupSwitch || (ins.isBranch() || ins.isJump())
                    && ins != repeatJump && (ins.getAddress() < loadAt && ins.getTargetAddress() > loadAt
                    || ins.getAddress() >= loadAt && ins.getTargetAddress() <= loadAt))
                throw new IllegalArgumentException(label + " unexpected control flow crossing portrait load guard");
        }
        if (repeatJump != null) {
            byte[] jump = instruction(Jump, repeatJump.operands[0] + guard.size()).getBytes();
            System.arraycopy(jump, 0, before, (int) repeatJump.getAddress(), jump.length);
        }
        var updated = new Code();
        updated.write(before, 0, loadAt);
        updated.writeBytes(guard.toByteArray());
        updated.write(before, loadAt, before.length - loadAt);
        body.setCodeBytes(updated.toByteArray());
        body.getCode().checkValidOffsets(body);
        System.out.printf("ChangePetView.%s: method %d, guarded request at 0x%x, fallback %d%n",
                name, body.method_info, loadAt, fallback);
    }
}
