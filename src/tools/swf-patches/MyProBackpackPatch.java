import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

public class MyProBackpackPatch {
    static final String SIMPLE = "mmo.petpackage.panel.SimpleViewController";
    static int member(ABC abc, String owner, String name) {
        return abc.instance_info.get(abc.findClassByName(owner)).instance_traits.traits.stream()
            .filter(t -> name.equals(abc.constants.getString(t.getName(abc).name_index)))
            .findFirst().orElseThrow().name_index;
    }
    static MethodBody method(ABC abc, String name) {
        var traits = abc.instance_info.get(abc.findClassByName(SIMPLE)).instance_traits.traits;
        var m = (TraitMethodGetterSetter)traits.stream().filter(t -> t.name_index == member(abc,SIMPLE,name))
            .findFirst().orElseThrow();
        return abc.findBody(m.method_info);
    }
    static void poster(ABC abc, boolean verify) throws Exception {
        var c = abc.constants;
        int helper = c.getQnameId("MyProBackpackMorph",Namespace.KIND_PACKAGE,"aola.tools",!verify);
        int pdi = member(abc,"mmo.petpackage.panel.inclusion.petdetailinfo.PetDetailBase","pdi");
        int layer = member(abc,SIMPLE,"_bg");
        int clear = member(abc,SIMPLE,"clearBg");
        for (var name : List.of("changeSkins","clearBg","refreshUI")) {
            boolean draw = name.equals("changeSkins");
            boolean refresh = name.equals("refreshUI");
            var body = method(abc,name);
            var code = body.getCode();
            int at = draw ? 6 : 2;
            require(code.code.get(0).definition.instructionCode == GetLocal0 && code.code.get(1).definition.instructionCode == PushScope);
            if (draw) {
                require(code.code.get(2).definition.instructionCode == GetLocal1 && code.code.get(3).definition.instructionCode == IfFalse);
                require(code.code.get(5).definition.instructionCode == CallPropVoid && code.code.get(5).operands[0] == member(abc,SIMPLE,"changeBg"));
            }
            var insert = new java.util.ArrayList<AVM2Instruction>();
            insert.add(op(GetLex,helper));
            int[] fields=draw ? new int[]{pdi,member(abc,SIMPLE,"_skinService"),layer,member(abc,SIMPLE,"_packageBG"),clear}
                : refresh ? new int[]{layer,pdi} : new int[]{layer};
            for (int field : fields) {
                insert.add(op(GetLocal0)); insert.add(op(GetProperty,field));
            }
            insert.add(op(CallPropVoid,c.getPublicQnameId(draw?"poster":refresh?"checkPoster":"cancelPoster",!verify),fields.length));
            if (verify) {
                for (int i=0;i<insert.size();i++) require(java.util.Arrays.equals(code.code.get(at+i).getBytes(),insert.get(i).getBytes()));
            } else {
                int i=at;
                for (var instruction : insert) code.insertInstruction(i++,instruction,body);
                body.max_stack = Math.max(body.max_stack,fields.length+1);
                if (draw) {
                    var branch = code.code.get(3);
                    branch.operands[0] = (int)(code.code.get(i).getAddress()-branch.getAddress()-branch.getBytesLength());
                }
                code.checkValidOffsets(body);
                body.setCode(code);
            }
            if (draw) {
                int label=at+insert.size();
                require(code.code.get(3).getTargetAddress()==code.code.get(label).getAddress() &&
                    code.code.get(label+1).operands[0]==member(abc,SIMPLE,"changeLabel"));
            }
        }
    }
    private static AVM2Instruction op(int opcode, int... operands) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[opcode], operands);
    }
    private static void require(boolean condition) {
        if (!condition) throw new IllegalArgumentException("Unrecognized backpack model or poster instruction layout");
    }
    private static void verify(Path candidate) throws Exception {
        try (var input = Files.newInputStream(candidate)) {
            var tags = new SWF(input, false).getAbcList();
            require(tags.size()==2 && tags.getFirst().getABC().findClassByName("aola.tools.MyProBackpackMorph")>=0);
            var abc = tags.getLast().getABC();
            poster(abc,true);
            var c = abc.constants;
            var owner = abc.findClassByName("mmo.petpackage.panel.inclusion.PetBigIconDetail");
            var m = abc.instance_info.get(owner).instance_traits.traits.stream()
                .filter(t -> c.getString(t.getName(abc).name_index).equals("afterOtherOperate")).toList();
            require(m.size()==1);
            var body=abc.findBody(((TraitMethodGetterSetter)m.getFirst()).method_info);
            var code=body.getCode().code;
            int bridge=c.getQnameId("MyProBackpackMorph",Namespace.KIND_PACKAGE,"aola.tools",false);
            var calls=code.stream().filter(i->i.definition.instructionCode==GetLex && i.operands[0]==bridge).toList();
            require(calls.size()==1);
            int p=code.indexOf(calls.getFirst());
            require(code.get(p+4).definition.instructionCode==CallProperty && code.get(p+4).operands[0]==c.getPublicQnameId("resolve",false)
                && code.get(p+4).operands[1]==1 && code.get(p+5).definition.instructionCode==Dup && code.get(p+6).definition.instructionCode==SetLocal);
            int local=code.get(p+6).operands[0];
            require(body.max_regs==local+1 && code.get(p+7).definition.instructionCode==CallProperty
                && code.get(p+7).operands[0]==c.getPublicQnameId("isBitMapType",false));
            var renders=code.stream().filter(i->i.definition.instructionCode==CallPropVoid && i.operands[0]==c.getPublicQnameId("setIcon",false)).toList();
            require(renders.size()==1);
            int r=code.indexOf(renders.getFirst());
            require(r>p+7 && code.get(r-5).definition.instructionCode==Pop && code.get(r-4).definition.instructionCode==GetLocal
                && code.get(r-4).operands[0]==local && renders.getFirst().operands[1]==3);
            body.getCode().checkValidOffsets(body);
            System.out.println("Read back helper.resolve, isBitMapType, setIcon(mapped,type,curPbi) from "+candidate);
            System.out.println("Read back SimpleViewController.changeSkins guarded refresh, clearBg and refreshUI poster hooks");
        }
    }
    public static void main(String[] args) throws Exception {
        if (args[0].equals("--verify")) { verify(Path.of(args[1])); return; }
        ABC abc;
        try (var input = Files.newInputStream(Path.of(args[0]))) {
            var tags = new SWF(input, false).getAbcList(); require(tags.size() == 1);
            abc = tags.getFirst().getABC();
        }
        try (var out = Files.newOutputStream(Path.of(args[1]))) { abc.saveToStream(out); }
        PetListSwitchPatch.apply(abc);
        PetTabSwitchPatch.apply(abc);
        PetActiveTabSwitchPatch.apply(abc);
        var c = abc.constants;
        var owner = abc.findClassByName("mmo.petpackage.panel.inclusion.PetBigIconDetail");
        require(owner >= 0);
        var matches = abc.instance_info.get(owner).instance_traits.traits.stream()
            .filter(t -> c.getString(t.getName(abc).name_index).equals("afterOtherOperate")).toList();
        require(matches.size() == 1);
        var body = abc.findBody(((TraitMethodGetterSetter) matches.getFirst()).method_info);
        var code = body.getCode();
        int viewId = c.getPublicQnameId("viewId", false);
        var reads = code.code.stream().filter(i -> i.definition.instructionCode == GetProperty && i.operands[0] == viewId).toList();
        require(reads.size() == 2 && body.max_regs == 2);
        for (var read : reads) {
            int pos = code.code.indexOf(read);
            var receiver = code.code.get(pos - 1);
            require(code.code.get(pos - 2).definition.instructionCode == GetLocal0 && receiver.definition.instructionCode == GetProperty
                && c.getString(c.getMultiname(receiver.operands[0]).name_index).equals("curPbi"));
        }
        var typeCall = code.code.get(code.code.indexOf(reads.getFirst()) + 1);
        var renderCall = code.code.get(code.code.indexOf(reads.getLast()) + 4);
        require(typeCall.definition.instructionCode == CallProperty && typeCall.operands[0] == c.getPublicQnameId("isBitMapType", false));
        require(renderCall.definition.instructionCode == CallPropVoid && renderCall.operands[0] == c.getPublicQnameId("setIcon", false) && renderCall.operands[1] == 3);
        // Read the official ID once for resolution and use the result for both
        // model family selection and the first setIcon argument. Leave the PBI intact.
        int slot = body.max_regs++;
        body.max_stack++;
        code.insertInstruction(code.code.indexOf(reads.getFirst()) - 2,
            op(GetLex, c.getQnameId("MyProBackpackMorph", Namespace.KIND_PACKAGE, "aola.tools", true)), body);
        int at = code.code.indexOf(reads.getFirst()) + 1;
        for (var i : List.of(op(CallProperty,c.getPublicQnameId("resolve", true),1), op(Dup), op(SetLocal,slot)))
            code.insertInstruction(at++,i,body);
        at = code.code.indexOf(reads.getLast()) + 1;
        code.insertInstruction(at++,op(Pop),body);
        code.insertInstruction(at,op(GetLocal,slot),body);
        code.checkValidOffsets(body);
        body.setCode(code);
        poster(abc,false);
        try (var out = Files.newOutputStream(Path.of(args[2]))) { abc.saveToStream(out); }
    }
}
