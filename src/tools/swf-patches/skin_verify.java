import java.io.ByteArrayOutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;

import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.types.MethodBody;

// Reconstruct the expected ABC from the original. Only declared method code, the
// two loop methods' register counts, and named public lookups may differ.
class SkinVerify {
    static void verifyLoop(MethodBody body) throws Exception {
        var code = body.getCode();
        int loops = 0;
        for (var op : code.code) {
            if (!op.definition.instructionName.equals("iftrue")) continue;
            long target = op.getTargetAddress();
            if (target >= op.getAddress()) continue;
            var destination = code.code.get(code.adr2pos(target));
            if (!destination.definition.instructionName.equals("label"))
                throw new IllegalArgumentException("Backward loop branch must target a label opcode");
            loops++;
        }
        if (loops != 1) throw new IllegalArgumentException("Expected one disposal loop");
    }

    static ABC read(String path) throws Exception {
        try (var stream = Files.newInputStream(Path.of(path))) {
            var tags = new SWF(stream, false).getAbcList();
            if (tags.size() != 1) throw new IllegalArgumentException("Expected one ABC");
            return tags.get(0).getABC();
        }
    }

    public static void main(String[] args) throws Exception {
        var expected = read(args[0]);
        var built = read(args[1]);
        var allowedNames = List.of("data", "masterList", "gc", "kill");
        for (int i = expected.constants.getMultinameCount(); i < built.constants.getMultinameCount(); i++) {
            int index = i;
            var name = allowedNames.stream()
                .filter(n -> built.constants.getPublicQnameId(n, false) == index)
                .findFirst().orElseThrow(() -> new IllegalArgumentException("Unexpected multiname " + index));
            if (expected.constants.getPublicQnameId(name, true) != i)
                throw new IllegalArgumentException("Unexpected constant order for " + name);
            System.out.println("Appended public name: " + name);
        }
        for (int arg = 3; arg < args.length; arg++) {
            int index = Integer.parseInt(args[arg]);
            var old = expected.bodies.get(index);
            var updated = built.bodies.get(index);
            if (index == 220 || index == 250) verifyLoop(updated);
            if (Arrays.equals(old.getCodeBytes(), updated.getCodeBytes()))
                throw new IllegalArgumentException("Expected changed method body " + index);
            old.setCodeBytes(updated.getCodeBytes());
            if (index == 220) old.max_regs = 3;
            if (index == 250) old.max_regs = 4;
        }
        var bytes = new ByteArrayOutputStream();
        expected.saveToStream(bytes);
        if (!Arrays.equals(bytes.toByteArray(), Files.readAllBytes(Path.of(args[2]))))
            throw new IllegalArgumentException("Other ABC bytes changed");
        System.out.println("Exact expected ABC. " + (args.length - 3) + " changed bodies out of " + built.bodies.size());
    }
}
