import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class MyProBattlePatch {
    static AVM2Instruction op(int code, int... args) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[code], args);
    }

    static MethodBody method(ABC abc,String owner,String name,int expected) {
        var traits=abc.instance_info.get(abc.findClassByName(owner)).instance_traits.traits;
        var methods=traits.stream().filter(t->abc.constants.getString(t.getName(abc).name_index).equals(name)).toList();
        if (methods.size()!=1 || !(methods.getFirst() instanceof TraitMethodGetterSetter m) || m.method_info!=expected)
            throw new IllegalArgumentException(owner+"."+name+" version mismatch");
        return abc.findBody(expected);
    }

    static int index(MethodBody body,int address,int opcode,int... operands) {
        var code=body.getCode().code;
        for(int n=0;n<code.size();n++) {
            var inst=code.get(n);
            if(inst.getAddress()==address && inst.definition==AVM2Code.instructionSet[opcode]
                    && (operands.length==0 || java.util.Arrays.equals(inst.operands,operands))) return n;
        }
        throw new IllegalArgumentException("Unexpected original instruction at "+address);
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
        if (original.size() + n != modified.size()) throw new IllegalArgumentException("Selector not persisted");
        for (int i = 0; i < n; i++)
            if (!Arrays.equals(added[i].getBytes(), modified.get(insertion+i).getBytes()))
                throw new IllegalArgumentException("Unexpected model selector");
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

    static void verify(Path basePath, Path resultPath) throws Exception {
        try (var baseStream = Files.newInputStream(basePath); var resultStream = Files.newInputStream(resultPath)) {
            var before = new SWF(baseStream, false).getAbcList().getFirst().getABC();
            var tags = new SWF(resultStream, false).getAbcList();
            if (tags.size() != 2 || tags.getFirst().getABC().findClassByName("aola.tools.MyProBattleModel") < 0)
                throw new IllegalArgumentException("Missing model definitions");
            var after = tags.getLast().getABC();
            var c = after.constants;
            int bridge=c.getQnameId("MyProBattleModel",Namespace.KIND_PACKAGE,"aola.tools",false);
            var owner = after.instance_info.get(after.findClassByName("mmo.petfightnew.common.MCPetContainer"));
            int face=owner.instance_traits.traits.stream().filter(t->"isFace".equals(c.getString(t.getName(after).name_index))).findFirst().orElseThrow().name_index;
            if (before.bodies.size() != after.bodies.size()) throw new IllegalArgumentException("Method count changed");
            for (var b : before.bodies) {
                var updated = after.findBody(b.method_info);
                if (b.method_info == 5172) {
                    validateSite(b,updated,index(b,80,ConvertI),op(GetLex,bridge),op(Swap),op(GetLocal0),op(GetProperty,face),op(CallProperty,c.getPublicQnameId("body",false),2));
                } else if (b.method_info == 50) {
                    validateSite(b,updated,index(b,10,ConvertI),op(GetLex,bridge),op(Swap),op(CallProperty,c.getPublicQnameId("icon",false),1));
                } else if (!Arrays.equals(b.getCodeBytes(),updated.getCodeBytes())) throw new IllegalArgumentException("Unexpected change to #"+b.method_info);
            }
            System.out.println("Verified persisted selectors 5172/50, every original jump and "+(before.bodies.size()-2)+" untouched methods");
        }
    }

    public static void main(String[] args) throws Exception {
        if (args[0].equals("--verify")) { verify(Path.of(args[1]),Path.of(args[2])); return; }
        ABC abc;
        try(var input=Files.newInputStream(Path.of(args[0]))) {
            var tags=new SWF(input,false).getAbcList();
            if(tags.size()!=1)throw new IllegalArgumentException("Unexpected battle ABCs");
            abc=tags.getFirst().getABC();
        }
        try(var out=Files.newOutputStream(Path.of(args[1]))){abc.saveToStream(out);}
        var c=abc.constants;
        int bridge=c.getQnameId("MyProBattleModel",Namespace.KIND_PACKAGE,"aola.tools",true);
        var body=method(abc,"mmo.petfightnew.common.MCPetContainer","startLoad",5172);
        int bodyAt=index(body,80,ConvertI);
        index(body,77,GetProperty,c.getPublicQnameId("flaId",false));
        index(body,72,IfNe,11);
        index(body,81,SetLocal,4);
        // isFace is a private trait. Reuse the QName belonging to this owner.
        var traits=abc.instance_info.get(abc.findClassByName("mmo.petfightnew.common.MCPetContainer")).instance_traits.traits;
        var field=traits.stream().filter(t->"isFace".equals(c.getString(t.getName(abc).name_index))).findFirst().orElseThrow();
        int face=field.name_index;
        insert(body,bodyAt,op(GetLex,bridge),op(Swap),op(GetLocal0),op(GetProperty,face),op(CallProperty,c.getPublicQnameId("body",true),2));
        body.max_stack=Math.max(body.max_stack,3);
        var icon=method(abc,"mmo.petfightnew.common.handler.PetHeadIconHandler","setIcon",50);
        int iconAt=index(icon,10,ConvertI);
        index(icon,6,CallProperty,c.getPublicQnameId("getPetFlaId",false),1);
        index(icon,11,SetLocal1);
        insert(icon,iconAt,op(GetLex,bridge),op(Swap),op(CallProperty,c.getPublicQnameId("icon",true),1));
        icon.max_stack=Math.max(icon.max_stack,2);
        try(var out=Files.newOutputStream(Path.of(args[2]))){abc.saveToStream(out);}
    }
}
