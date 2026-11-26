import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Set;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitClass;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class PetListProbeAccess {
    public static void main(String[] args) throws Exception {
        try (var input = Files.newInputStream(Path.of(args[0])); var out = Files.newOutputStream(Path.of(args[1]))) {
            var abc = new SWF(input, false).getAbcList().getFirst().getABC();
            var c = abc.constants;
            int list = abc.findClassByName("mmo.petpackage.panel.inclusion.PetList");
            int slot = abc.findClassByName("mmo.petpackage.panel.inclusion.petlist.PetSingleIconBase");
            var owners = List.of(list, slot);
            abc.script_info.removeIf(s -> s.traits.traits.stream().noneMatch(t -> t instanceof TraitClass cls
                    && owners.contains(cls.class_info)));
            if (abc.script_info.size() != 2) throw new IllegalArgumentException("Expected two single-class scripts");
            for (var script : abc.script_info) {
                var def = (TraitClass)script.traits.traits.getFirst();
                int owner = def.class_info;
                def.class_info = owners.indexOf(owner);
                var init = abc.findBody(script.init_index);
                var code = init.getCode();
                var definitions = code.code.stream().filter(i -> i.definition.instructionCode == NewClass
                        && i.operands[0] == owner).toList();
                if (definitions.size() != 1) throw new IllegalArgumentException("Unexpected class initializer");
                definitions.getFirst().operands[0] = def.class_info;
                init.setCode(code);
                var instance = abc.instance_info.get(owner);
                // The fixture supplies its own timeline and services. All
                // methods that handle the refresh and selection keep their code.
                abc.findBody(instance.iinit_index).setCodeBytes(new byte[]{
                    (byte)GetLocal0, (byte)PushScope, (byte)GetLocal0, (byte)ConstructSuper, 0, (byte)ReturnVoid});
                var expose = owner == list
                    ? Set.of("pets", "_objs", "_mcPetIconFullScreen", "onPMDetailUpdate", "setSelectedPet", "tempPBI", "ipds")
                    : Set.of("mcHp", "mcNormal", "mcSelect", "mcAdd", "_mcDetail");
                var keep = owner == list
                    ? Set.of("refreshUI", "refreshMcPetIconFullScreen", "_selectedPet", "selectedPet", "dispose")
                    : Set.of("_objData", "objData", "id", "nullPet", "setData", "updateUI", "selected", "dispose", "mcPetLabel");
                instance.instance_traits.traits.removeIf(t -> {
                    var q = t.getName(abc);
                    var name = c.getString(q.name_index);
                    if (expose.contains(name)) {
                        q.namespace_index = c.getNamespaceId(Namespace.KIND_PACKAGE, "", 0, false);
                        return false;
                    }
                    return !keep.contains(name);
                });
            }
            var instances = owners.stream().map(abc.instance_info::get).toList();
            var classes = owners.stream().map(abc.class_info::get).toList();
            abc.instance_info.clear();
            abc.instance_info.addAll(instances);
            abc.class_info.clear();
            abc.class_info.addAll(classes);
            abc.saveToStream(out);
        }
    }
}
