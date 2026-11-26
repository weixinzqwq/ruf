import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class MyProLineupPatch {
    static final String OWNER = "mmo.petpackage.panel.PetPackagePanel";
    static ABC game(String path) throws Exception {
        try (var in=Files.newInputStream(Path.of(path))) {
            var abcs = new SWF(in,false).getAbcList().stream().map(t->t.getABC())
                    .filter(a->a.findClassByName(OWNER)>=0).toList();
            if(abcs.size()!=1) throw new IllegalArgumentException("Ambiguous backpack ABC");
            return abcs.getFirst();
        }
    }
    static AVM2Instruction op(int code,int... args){return new AVM2Instruction(0,AVM2Code.instructionSet[code],args);}
    static void patch(ABC a) throws Exception {
        var c=a.constants;
        var traits=a.instance_info.get(a.findClassByName(OWNER)).instance_traits.traits;
        var helper=c.getQnameId("MyProLineup",Namespace.KIND_PACKAGE,"aola.tools",true);
        for(var name:List.of("setData","disposeEvent")){
            boolean bind=name.equals("setData");
            var matches=traits.stream().filter(t->c.getString(t.getName(a).name_index).equals(name)).toList();
            if(matches.size()!=1 || !(matches.getFirst() instanceof TraitMethodGetterSetter m))
                throw new IllegalArgumentException("Missing backpack method "+name);
            var b=a.findBody(m.method_info);
            var code=b.getCode();
            if(a.method_info.get(m.method_info).param_types.length!=(bind?3:0)
                || code.code.get(0).definition.instructionCode!=GetLocal0 || code.code.get(1).definition.instructionCode!=PushScope)
                throw new IllegalArgumentException("Unrecognized backpack method "+name);
            if(bind && code.code.stream().noneMatch(i->i.definition.instructionCode==SetProperty
                    && "_objs".equals(c.getString(c.getMultiname(i.operands[0]).name_index))))
                throw new IllegalArgumentException("setData no longer sets the official backpack list");
            int pos=2;
            code.insertInstruction(pos++,op(GetLex,helper),b);
            code.insertInstruction(pos++,op(GetLocal0),b);
            if(bind) code.insertInstruction(pos++,op(GetLocal1),b);
            code.insertInstruction(pos,op(CallPropVoid,c.getPublicQnameId(bind?"bind":"unbind",true),bind?2:1),b);
            code.checkValidOffsets(b); b.max_stack=Math.max(b.max_stack,bind?3:2); b.setCode(code);
        }
    }
    public static void main(String[] args) throws Exception {
        var original=game(args[0]);
        if(args.length==2){
            patch(original); var candidate=game(args[1]);
            if(original.bodies.size()!=candidate.bodies.size())throw new IllegalArgumentException("Method count changed");
            for(var b:original.bodies) if(!Arrays.equals(b.getCodeBytes(),candidate.findBody(b.method_info).getCodeBytes()))
                throw new IllegalArgumentException("Lineup method readback failed: "+b.method_info);
        }else{
            try(var out=Files.newOutputStream(Path.of(args[1]))){original.saveToStream(out);}
            patch(original);
            try(var out=Files.newOutputStream(Path.of(args[2]))){original.saveToStream(out);}
        }
    }
}
