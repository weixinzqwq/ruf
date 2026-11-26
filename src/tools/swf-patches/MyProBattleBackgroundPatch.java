import java.nio.file.*;
import java.util.*;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class MyProBattleBackgroundPatch {
    static final String OWNER = "mmo.petfightnew.common.handler.BackGroundViewHandler";
    static final String HELPER = "aola.tools.MyProBattleBackground";
    static ABC find(SWF swf, String name) {
        var matches = swf.getAbcList().stream().map(t -> t.getABC()).filter(a -> a.findClassByName(name) >= 0).toList();
        if (matches.size() != 1) throw new IllegalArgumentException(name + " not unique");
        return matches.getFirst();
    }
    static SWF read(String path) throws Exception {
        try (var file = Files.newInputStream(Path.of(path))) { return new SWF(file,false); }
    }
    static int trait(ABC a, String name, boolean method) {
        var traits = a.instance_info.get(a.findClassByName(OWNER)).instance_traits.traits;
        var matches = traits.stream().filter(t -> name.equals(a.constants.getString(t.getName(a).name_index))).toList();
        if (matches.size()!=1) throw new IllegalArgumentException("Missing or ambiguous " + name);
        var t = matches.getFirst();
        if (method && t instanceof TraitMethodGetterSetter m) return m.method_info;
        if (!method && !(t instanceof TraitMethodGetterSetter)) return t.name_index;
        throw new IllegalArgumentException("Unexpected trait type: "+name);
    }
    static AVM2Instruction op(int code, int... args) {
        return new AVM2Instruction(0,AVM2Code.instructionSet[code],args);
    }
    static void hook(MethodBody b, int pos, int helper, int method, int... fields) throws Exception {
        var code = b.getCode();
        code.insertInstruction(pos++,op(GetLex,helper),b);
        for (int field:fields) {
            code.insertInstruction(pos++,op(GetLocal0),b);
            code.insertInstruction(pos++,op(GetProperty,field),b);
        }
        code.insertInstruction(pos,op(CallPropVoid,method,fields.length),b);
        code.checkValidOffsets(b);
        b.setCode(code);
        b.max_stack = Math.max(b.max_stack, fields.length+1);
    }
    static void shape(MethodBody body) {
        var ops = body.getCode().code;
        if (body.exceptions.length!=0 || ops.size()<4 || ops.getFirst().definition!=AVM2Code.instructionSet[GetLocal0]
                || ops.get(1).definition!=AVM2Code.instructionSet[PushScope] || ops.getLast().definition!=AVM2Code.instructionSet[ReturnVoid])
            throw new IllegalArgumentException("Background method layout changed: " + body.method_info);
    }
    static void apply(ABC a) throws Exception {
        var c = a.constants;
        int cls = c.getQnameId("MyProBattleBackground",Namespace.KIND_PACKAGE,"aola.tools",true);
        int layer=trait(a,"_layerBackGround",false), platform=trait(a,"_layerPlatform",false);
        var scene=a.findBody(trait(a,"setSceneBackGround",true));
        var dispose=a.findBody(trait(a,"dispose",true));
        var bs=a.findBody(trait(a,"setBS",true));
        for (var b:List.of(scene,dispose,bs)) shape(b);
        var ops=bs.getCode().code;
        var lastCall=ops.get(ops.size()-2);
        if (lastCall.definition!=AVM2Code.instructionSet[CallPropVoid] || lastCall.operands[0]!=c.getPublicQnameId("refreshBg",false))
            throw new IllegalArgumentException("Expected final skill background refresh");
        hook(bs,ops.size()-1,cls,c.getPublicQnameId("afterBS",true),layer);
        hook(bs,2,cls,c.getPublicQnameId("beforeBS",true),layer);
        hook(scene,2,cls,c.getPublicQnameId("bind",true),layer,platform);
        hook(dispose,2,cls,c.getPublicQnameId("detach",true),layer);
    }
    static void save(ABC a,String path) throws Exception {
        try(var out=Files.newOutputStream(Path.of(path))){a.saveToStream(out);}
    }
    public static void main(String[] args) throws Exception {
        if ("--verify".equals(args[0])) {
            var original=find(read(args[1]),OWNER);
            var swf=read(args[2]);
            find(swf,HELPER);
            var updated=find(swf,OWNER);
            // Reproduce the anchored insertions and compare the persisted bodies, including every unrelated method.
            apply(original);
            if(original.bodies.size()!=updated.bodies.size()) throw new IllegalArgumentException("Method count mismatch");
            for(var body:original.bodies) if(!Arrays.equals(body.getCodeBytes(),updated.findBody(body.method_info).getCodeBytes()))
                throw new IllegalArgumentException("Background read-back failed: "+body.method_info);
            return;
        }
        var a=find(read(args[0]),OWNER);
        save(a,args[1]);
        apply(a);
        save(a,args[2]);
    }
}
