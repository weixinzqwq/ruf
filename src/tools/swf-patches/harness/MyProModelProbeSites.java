import java.io.ByteArrayOutputStream;
import java.nio.file.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.*;
import com.jpexs.decompiler.flash.abc.types.traits.*;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

// Lift the exact serialized selector instructions into independent test methods.
// Pool indices are local to each ABC. The fixture supplies a public isFace field
// for the private field normally owned by MCPetContainer.
class MyProModelProbeSites {
    static void lift(ABC source, ABC fixture, boolean icon) throws Exception {
        String method = icon ? "icon" : "body";
        var owner = fixture.instance_info.get(fixture.findClassByName("probe.GameSites"));
        var trait = owner.instance_traits.traits.stream().filter(t -> method.equals(
                fixture.constants.getString(t.getName(fixture).name_index))).findFirst().orElseThrow();
        var target = fixture.findBody(((TraitMethodGetterSetter)trait).method_info);
        var from = source.findBody(icon ? 50 : 5172).getCode().code;
        var bytes = new ByteArrayOutputStream();
        bytes.write(GetLocal0);
        bytes.write(PushScope);
        int count = 0;
        for (var ins : from) {
            if (ins.getAddress() < (icon ? 2 : 76)) continue;
            int opcode = ins.definition.instructionCode;
            int[] args = ins.operands == null ? null : ins.operands.clone();
            if (opcode == GetProperty || opcode == CallProperty || opcode == GetLex) {
                var multiname = source.constants.getMultiname(args[0]);
                var name = source.constants.getString(multiname.name_index);
                if (opcode == GetLex) {
                    var ns = multiname.getNamespace(source.constants);
                    args[0] = fixture.constants.getQnameId(name, Namespace.KIND_PACKAGE, ns.getName(source.constants).toRawString(), true);
                } else args[0] = fixture.constants.getPublicQnameId(name, true);
            }
            bytes.writeBytes(new AVM2Instruction(0,AVM2Code.instructionSet[opcode],args).getBytes());
            count++;
            if (opcode == ConvertI) break;
        }
        if (count != (icon ? 7 : 8)) throw new IllegalArgumentException("Unexpected selector length: "+method+" "+count);
        bytes.write(ReturnValue);
        target.max_stack = 3;
        target.setCodeBytes(bytes.toByteArray());
    }

    public static void main(String[] args) throws Exception {
        try (var gameIn=Files.newInputStream(Path.of(args[0])); var probeIn=Files.newInputStream(Path.of(args[1]));
             var output=Files.newOutputStream(Path.of(args[2]))) {
            var game=new SWF(gameIn,false).getAbcList().getLast().getABC();
            var probe=new SWF(probeIn,false).getAbcList().getFirst().getABC();
            lift(game,probe,false);
            lift(game,probe,true);
            probe.saveToStream(output);
        }
    }
}
