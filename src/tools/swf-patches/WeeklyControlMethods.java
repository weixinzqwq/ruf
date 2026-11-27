import java.nio.file.Files;
import java.nio.file.Path;

import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;

/** Resolve method body indices against the same ABC used for the p-code export. */
class WeeklyControlMethods {
    public static void main(String[] args) throws Exception {
        try (var input = Files.newInputStream(Path.of(args[0]))) {
            var blocks = new SWF(input, false).getAbcList();
            if (blocks.size() != 1) {
                throw new IllegalArgumentException("Expected one control ABC block");
            }
            var abc = blocks.getFirst().getABC();
            for (var target : Files.readAllLines(Path.of(args[1]))) {
                var parts = target.split("\t", 2);
                var owner = parts[0];
                var method = parts[1];
                int ownerIndex = abc.findClassByName(owner);
                if (ownerIndex < 0) {
                    throw new IllegalArgumentException("Missing class " + owner);
                }
                var instance = abc.instance_info.get(ownerIndex);
                int methodIndex = instance.iinit_index;
                if (!method.equals("$iinit")) {
                    var methods = instance.instance_traits.traits.stream()
                        .filter(trait -> trait instanceof TraitMethodGetterSetter)
                        .map(trait -> (TraitMethodGetterSetter) trait)
                        .filter(trait -> method.equals(abc.constants.getString(trait.getName(abc).name_index)))
                        .toList();
                    if (methods.size() != 1) {
                        throw new IllegalArgumentException("Missing or ambiguous method " + owner + "." + method);
                    }
                    methodIndex = methods.getFirst().method_info;
                }
                int bodyIndex = abc.findBodyIndex(methodIndex);
                if (bodyIndex < 0) {
                    throw new IllegalArgumentException("Missing method body " + owner + "." + method);
                }
                System.out.println(bodyIndex);
            }
        }
    }
}
