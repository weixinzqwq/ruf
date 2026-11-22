import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.types.Namespace;

// Expose the existing private slots to the test movie, and redirect the host
// QName to a test double. This never changes a single method's bytecode.
class NoticeProbeAccess {
    public static void main(String[] args) throws Exception {
        try (var input = Files.newInputStream(Path.of(args[0]));
             var output = Files.newOutputStream(Path.of(args[1]))) {
            var abc = new SWF(input, false).getAbcList().getFirst().getABC();
            var c = abc.constants;
            var host = c.getMultiname(c.getQnameId("ExternalInterface", Namespace.KIND_PACKAGE, "flash.external", false));
            host.namespace_index = c.getNamespaceId(Namespace.KIND_PACKAGE, "probe", 0, true);
            host.name_index = c.getStringId("NoticeHost", true);
            for (String owner : List.of("MessageRandomShow", "MessageSendService")) {
                var traits = abc.instance_info.get(abc.findClassByName("mmo.MessageSend." + owner)).instance_traits;
                for (var trait : traits.traits) {
                    var qname = trait.getName(abc);
                    if (List.of("sTimer", "showTimer", "onSystemMsgComes", "onHonorMsgDispatch",
                            "onTitleHandle", "onReturnBroadcast", "messageComing",
                            "messages", "messageShowing").contains(c.getString(qname.name_index)))
                        qname.namespace_index = c.getNamespaceId(Namespace.KIND_PACKAGE, "", 0, false);
                }
            }
            abc.saveToStream(output);
        }
    }
}
