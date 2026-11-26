import com.jpexs.decompiler.flash.SWF;
import java.nio.file.Files;
import java.nio.file.Path;

// Only redirect ExternalInterface in the isolated probe, retaining the revision and scale bytecode.
class MyProBackpackHost {
    public static void main(String[] args) throws Exception {
        try (var in=Files.newInputStream(Path.of(args[0])); var out=Files.newOutputStream(Path.of(args[1]))) {
            var abc=new SWF(in,false).getAbcList().getFirst().getABC();
            if(abc.findClassByName("aola.tools.MyProBackpackMorph")<0) throw new IllegalArgumentException("Missing revision helper");
            var c=abc.constants;
            c.setString(c.getStringId("ExternalInterface",false),"ModelHost");
            c.setString(c.getStringId("flash.external",false),"probe");
            abc.saveToStream(out);
        }
    }
}
