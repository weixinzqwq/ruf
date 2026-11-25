import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import java.nio.file.Files;
import java.nio.file.Path;

/** Rebuild the backpack ABC while retaining its existing class and resource bindings. */
public class PetSwitchPatch {
    private static void save(ABC abc, String file) throws Exception {
        try (var out = Files.newOutputStream(Path.of(file))) {
            abc.saveToStream(out);
        }
    }

    public static void main(String[] args) throws Exception {
        ABC abc;
        try (var in = Files.newInputStream(Path.of(args[0]))) {
            var tags = new SWF(in, false).getAbcList();
            if (tags.size() != 1) throw new IllegalArgumentException("Expected one backpack DoABC");
            abc = tags.getFirst().getABC();
        }
        save(abc, args[1]);
        PetListSwitchPatch.apply(abc);
        PetTabSwitchPatch.apply(abc);
        PetActiveTabSwitchPatch.apply(abc);
        save(abc, args[2]);
    }
}
