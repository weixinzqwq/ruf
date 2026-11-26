import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

public class MyProBackpackPatch {
    private static final String DETAIL = "mmo.petpackage.panel.inclusion.PetBigIconDetail";
    private static final String HELPER = "aola.tools.MyProBackpackMorph";

    private static int member(ABC abc, String name) {
        int owner = abc.findClassByName(DETAIL);
        require(owner >= 0);
        return abc.instance_info.get(owner).instance_traits.traits.stream()
            .filter(t -> name.equals(abc.constants.getString(t.getName(abc).name_index)))
            .findFirst().orElseThrow().name_index;
    }

    private static MethodBody method(ABC abc, String name) {
        int trait = member(abc, name);
        var traits = abc.instance_info.get(abc.findClassByName(DETAIL)).instance_traits.traits;
        var method = (TraitMethodGetterSetter) traits.stream().filter(t -> t.name_index == trait)
            .findFirst().orElseThrow();
        return abc.findBody(method.method_info);
    }

    private static void revision(ABC abc, boolean verify) throws Exception {
        var c = abc.constants;
        int helper = c.getQnameId("MyProBackpackMorph", Namespace.KIND_PACKAGE, "aola.tools", !verify);
        int refresh = c.getPublicQnameId("refresh", !verify);
        int preSkin = member(abc, "_preSkinId");
        int getSkin = member(abc, "getSkinId");
        var body = method(abc, "updatePetIcon");
        var code = body.getCode();
        require(code.code.get(0).definition.instructionCode == GetLocal0
            && code.code.get(1).definition.instructionCode == PushScope);

        // A changed revision invalidates the same-skin shortcut. The ordinary
        // update path clears the current icon and mounts it with the original ID.
        var invalidate = List.of(op(GetLocal0), op(PushByte, -1), op(SetProperty, preSkin));
        int skip = invalidate.stream().mapToInt(AVM2Instruction::getBytesLength).sum();
        var hook = List.of(op(GetLex, helper), op(CallProperty, refresh, 0), op(IfFalse, skip),
            invalidate.get(0), invalidate.get(1), invalidate.get(2));
        int at = 2;
        int nativeStart = verify ? at + hook.size() : at;
        require(code.code.get(nativeStart).definition.instructionCode == GetLocal0
            && code.code.get(nativeStart + 1).definition.instructionCode == GetLocal1
            && code.code.get(nativeStart + 2).definition.instructionCode == CallProperty
            && code.code.get(nativeStart + 2).operands[0] == getSkin);
        if (verify) {
            for (var instruction : hook) require(Arrays.equals(code.code.get(at++).getBytes(), instruction.getBytes()));
            require(code.code.get(4).getTargetAddress() == code.code.get(nativeStart).getAddress());
        } else {
            for (int i = hook.size() - 1; i >= 0; i--) code.insertInstruction(at, hook.get(i), body);
            body.max_stack = Math.max(body.max_stack, 2);
            body.setCode(code);
        }
        code.checkValidOffsets(body);
    }

    private static void scale(ABC abc, boolean verify) throws Exception {
        var c = abc.constants;
        int helper = c.getQnameId("MyProBackpackMorph", Namespace.KIND_PACKAGE, "aola.tools", !verify);
        int scaleOnLoaded = c.getPublicQnameId("scaleOnLoaded", !verify);
        int setIcon = c.getPublicQnameId("setIcon", false);
        int viewId = c.getPublicQnameId("viewId", false);
        int handler = member(abc, "_petIconHandler");
        int pet = member(abc, "curPbi");
        var body = method(abc, "afterOtherOperate");
        var code = body.getCode();
        var nativeArgs = List.of(op(GetLocal0), op(GetProperty, handler),
            op(GetLocal0), op(GetProperty, pet), op(GetProperty, viewId), op(GetLocal1),
            op(GetLocal0), op(GetProperty, pet));
        var callback = List.of(op(GetLex, helper), op(GetLocal0), op(GetProperty, handler),
            op(GetLocal0), op(GetProperty, pet), op(GetProperty, viewId), op(GetLocal1),
            op(CallProperty, scaleOnLoaded, 3));
        int call = -1;
        for (int i = 0; i < code.code.size(); i++) {
            var ins = code.code.get(i);
            if (ins.definition.instructionCode == CallPropVoid && ins.operands[0] == setIcon) {
                require(call == -1 && ins.operands[1] == (verify ? 4 : 3));
                call = i;
            }
        }
        int at = call - nativeArgs.size() - (verify ? callback.size() : 0);
        require(at >= 0);
        for (var ins : nativeArgs) require(Arrays.equals(code.code.get(at++).getBytes(), ins.getBytes()));
        if (verify) {
            for (var ins : callback) require(Arrays.equals(code.code.get(at++).getBytes(), ins.getBytes()));
            require(at == call && body.max_stack >= 8);
        } else {
            // Keep the existing arguments and pass the scale callback to
            // setIcon, which runs it once the native model is laid out.
            code.code.get(call).operands[1] = 4;
            for (int i = callback.size() - 1; i >= 0; i--) {
                code.insertInstruction(call, callback.get(i), body);
            }
            body.max_stack = Math.max(body.max_stack, 8);
            body.setCode(code);
        }
        code.checkValidOffsets(body);
    }

    private static AVM2Instruction op(int opcode, int... operands) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[opcode], operands);
    }

    private static void require(boolean condition) {
        if (!condition) throw new IllegalArgumentException("Unrecognized backpack hook layout");
    }

    private static void verify(Path candidate) throws Exception {
        try (var input = Files.newInputStream(candidate)) {
            var tags = new SWF(input, false).getAbcList();
            require(tags.stream().anyMatch(tag -> tag.getABC().findClassByName(HELPER) >= 0));
            var game = tags.stream().map(tag -> tag.getABC())
                .filter(abc -> abc.findClassByName(DETAIL) >= 0).findFirst().orElseThrow();
            revision(game, true);
            scale(game, true);
            System.out.println("Verified backpack revision and scale callbacks in " + candidate);
        }
    }

    public static void main(String[] args) throws Exception {
        if (args[0].equals("--verify")) { verify(Path.of(args[1])); return; }
        ABC abc;
        try (var input = Files.newInputStream(Path.of(args[0]))) {
            var tags = new SWF(input, false).getAbcList();
            require(tags.size() == 1);
            abc = tags.getFirst().getABC();
        }
        try (var out = Files.newOutputStream(Path.of(args[1]))) { abc.saveToStream(out); }
        PetListSwitchPatch.apply(abc);
        PetTabSwitchPatch.apply(abc);
        PetActiveTabSwitchPatch.apply(abc);
        revision(abc, false);
        scale(abc, false);
        try (var out = Files.newOutputStream(Path.of(args[2]))) { abc.saveToStream(out); }
    }
}
