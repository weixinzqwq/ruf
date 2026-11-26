import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;

class MyProBattlePatch {
    private static final String BODY = "mmo.petfightnew.common.MCPetContainer";
    private static final String ICON = "mmo.petfightnew.common.handler.PetHeadIconHandler";

    static AVM2Instruction op(int code, int... args) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[code], args);
    }

    static void insert(MethodBody body, int index, AVM2Instruction... added) throws Exception {
        var code = body.getCode();
        for (var instruction : added) code.insertInstruction(index++, instruction, body);
        code.checkValidOffsets(body);
        body.setCode(code);
    }

    static void validateSite(MethodBody before, MethodBody after, int insertion, AVM2Instruction... added) {
        var original = before.getCode().code;
        var modified = after.getCode().code;
        int n = added.length;
        if (original.size() + n != modified.size()) throw new IllegalArgumentException("Insertion not persisted");
        for (int i = 0; i < n; i++)
            if (!Arrays.equals(added[i].getBytes(), modified.get(insertion+i).getBytes()))
                throw new IllegalArgumentException("Unexpected inserted instruction");
        for (int i = 0; i < original.size(); i++) {
            var a = original.get(i);
            var b = modified.get(i < insertion ? i : i+n);
            if (a.definition != b.definition) throw new IllegalArgumentException("Original instruction changed");
            if (a.isBranch() || a.isJump()) {
                int target = original.indexOf(original.stream().filter(x -> x.getAddress() == a.getTargetAddress()).findFirst().orElseThrow());
                if (b.getTargetAddress() != modified.get(target < insertion ? target : target+n).getAddress())
                    throw new IllegalArgumentException("Original jump destination changed");
            } else if (!Arrays.equals(a.getBytes(), b.getBytes())) throw new IllegalArgumentException("Original operand changed");
        }
    }

    private static ABC game(Path movie) throws Exception {
        try (var input = Files.newInputStream(movie)) {
            var tags = new SWF(input, false).getAbcList();
            if (tags.stream().anyMatch(tag -> tag.getABC().findClassByName("aola.tools.MyProBattleModel") >= 0))
                throw new IllegalArgumentException("Old battle model helper still present");
            return tags.stream().map(tag -> tag.getABC())
                .filter(abc -> abc.findClassByName(BODY) >= 0).findFirst().orElseThrow();
        }
    }

    private static MethodBody method(ABC abc, String owner, String name) {
        var traits = abc.instance_info.get(abc.findClassByName(owner)).instance_traits.traits;
        var method = (TraitMethodGetterSetter) traits.stream()
            .filter(t -> name.equals(abc.constants.getString(t.getName(abc).name_index)))
            .findFirst().orElseThrow();
        return abc.findBody(method.method_info);
    }

    private static void verify(ABC baseline, ABC candidate, String owner, String name) {
        if (!Arrays.equals(method(baseline, owner, name).getCodeBytes(), method(candidate, owner, name).getCodeBytes()))
            throw new IllegalArgumentException("Official model selection changed in " + owner + "." + name);
    }

    public static void main(String[] args) throws Exception {
        var baseline = game(Path.of(args[0]));
        var candidate = game(Path.of(args[1]));
        verify(baseline, candidate, BODY, "startLoad");
        verify(baseline, candidate, ICON, "setIcon");
        System.out.println("Verified official model and avatar selection in " + args[1]);
    }
}
