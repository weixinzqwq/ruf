import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.function.Predicate;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

final class MyProBackpackVisualPatch {
    private static final String PANEL = "mmo.petpackage.panel.";
    private static final String VIEW = PANEL + "SimpleViewController";
    private static final String DETAIL = PANEL + "inclusion.PetBigIconDetail";
    private static final String CONTROLS = PANEL + "inclusion.PackageControlBar";
    private static final String HEAD = PANEL + "inclusion.petlist.PetSingleIconDetail";

    private final ABC abc;
    private final boolean verify;
    private final int helper;
    private final int viewId;
    private final int diyId;

    private MyProBackpackVisualPatch(ABC abc, boolean verify) {
        this.abc = abc;
        this.verify = verify;
        helper = abc.constants.getQnameId("MyProBackpackMorph", Namespace.KIND_PACKAGE, "aola.tools", !verify);
        viewId = abc.constants.getPublicQnameId("viewId", false);
        diyId = abc.constants.getPublicQnameId("useDIYBgId", false);
    }

    static void apply(ABC abc, boolean verify) throws Exception {
        var patch = new MyProBackpackVisualPatch(abc, verify);
        patch.view();
        patch.detail();
        patch.controls();
        patch.head();
    }

    private void view() throws Exception {
        int skin = member(VIEW, "skinId");
        int service = member(VIEW, "_skinService");
        int bg = member(VIEW, "_bg");
        int pdi = abc.constants.getQnameId("pdi", Namespace.KIND_PROTECTED,
            PANEL + "inclusion.petdetailinfo:PetDetailBase", false);
        var pet = field(pdi);
        var mapped = visualId(service, pet);

        for (String name : List.of("isHuandong", "refreshSkin", "isHasBg", "isOpenHaiBao", "changeLabel")) {
            int count = name.equals("changeLabel") ? 3 : name.equals("isOpenHaiBao") ? 2 : 1;
            inject(VIEW, name, named(GetProperty, skin), true, count, mapped);
        }
        // The first skinId in needShowBtn is a PMData lookup. Only the
        // later skin-service metadata query can accept a special skin ID.
        inject(VIEW, "needShowBtn", named(GetProperty, skin), true, 2, null, mapped);
        inject(VIEW, "changeBg", named(GetProperty, skin), true, 5, mapped);
        var particle = body(VIEW, "changeBg").getCode().code.stream()
            .filter(opcode(NewFunction)).findFirst().orElseThrow();
        var particleBody = abc.findBody(particle.operands[0]);
        int self = particleBody.getCode().code.stream().filter(named(GetLex, "self"))
            .findFirst().orElseThrow().operands[0];
        inject(particleBody, VIEW, "changeBg particle", named(GetLex, skin), true, 1,
            visualId(List.of(op(GetLex, self), op(GetProperty, service)),
                List.of(op(GetLex, self), op(GetProperty, pdi))));
        inject(VIEW, "changeBg", named(GetProperty, diyId), true, 2,
            decorate("useDIYBackground", 3, field(service), pet));
        inject(VIEW, "isHasBg", named(GetProperty, diyId), true, 1,
            decorate("useDIYBackground", 3, field(service), pet));
        inject(VIEW, "changeBg", named(CallProperty, "isOpenPetPackageState"), true, 1,
            state(service, pet, true));
        inject(VIEW, "isOpenHaiBao", named(CallProperty, "isOpenPetPackageState"), true, 1,
            state(service, pet, true));
        inject(VIEW, "showPreMovie", named(GetProperty, skin), true, 4, mapped,
            decorate("preMoviePreference", 3, field(service), pet), null, mapped);
        inject(VIEW, "showPreMovie", opcode(NewFunction), true, 1,
            decorate("guardBackground", 2, field(bg), List.of(op(Swap))));
        inject(VIEW, "showPreMovie", named(CallProperty, "isOpenPetPackageState"), true, 1,
            state(service, pet, false));
        inject(VIEW, "addDIYBg", named(GetProperty, member(VIEW, "onGetDIYBg")), true, 1,
            decorate("guardBackground", 2, field(bg), List.of(op(Swap))));
        inject(VIEW, "refreshSkin", named(CallProperty, member(VIEW, "isNothingChange")), true, 1,
            decorate("unchangedSelection", 1));
        inject(VIEW, "refreshSkin", named(CallPropVoid, "stopPetSkinBackgroundMusic"), true, 1,
            join(callVoid("stopTalk", 0), List.of(op(GetLocal0), op(CallPropVoid, member(VIEW, "clearBg"), 0))));
        inject(VIEW, "clearBg", opcode(PushScope), true, 1,
            callVoid("beginBackground", 1, field(bg)));
        int displayUtil = abc.constants.getQnameId("DisplayUtil", Namespace.KIND_PACKAGE, "mmo.common", false);
        int stopAndRemove = abc.constants.getPublicQnameId("stopAndRemove", !verify);
        // stopAndRemoveChild leaves the particle sprite itself attached.
        inject(VIEW, "clearCommonLiZiMv", named(CallPropVoid, "stopAndRemoveChild"), true, 1,
            List.of(op(GetLex, displayUtil), op(GetLocal2), op(CallPropVoid, stopAndRemove, 1)));
        inject(VIEW, "setBackgroundContainer", opcode(ReturnVoid), false, 1,
            callVoid("registerView", 2, List.of(op(GetLocal0)), field(member(VIEW, "refreshSkin"))));
        inject(VIEW, "dispose", opcode(PushScope), true, 1,
            callVoid("unregisterView", 1, List.of(op(GetLocal0))));
        inject(VIEW, "dispose", named(CallPropVoid, member(VIEW, "clearBg")), true, 1,
            callVoid("forgetBackground", 1, field(bg)));
    }

    private void detail() throws Exception {
        int service = member(DETAIL, "_skinDataService");
        var pet = List.of(op(GetLocal1));
        // getSkinId only feeds view metadata; afterOtherOperate still sets
        // the native icon from curPbi.viewId for HTTP replacement and HD scaling.
        inject(DETAIL, "getSkinId", opcode(ReturnValue), false, 2, visualId(service, pet));
        inject(DETAIL, "updatePetIcon", named(CallProperty, member(DETAIL, "getSkinId")), true, 4,
            callVoid("registerIcon", 2, field(member(DETAIL, "tryDisposePetIcon")),
                field(member(DETAIL, "onUpdateHaibao"))), null, null, null);
        inject(DETAIL, "updatePetIcon", named(CallProperty, "isOpenPetPackageState"), true, 1,
            state(service, pet, false));
        inject(DETAIL, "updatePetIcon", named(CallProperty, "needShowPetView"), true, 1,
            decorate("showPetModel", 3, field(service), pet));
        inject(DETAIL, "updatePetIcon", named(GetProperty, diyId), true, 1,
            decorate("useDIYBackground", 3, field(service), pet));
        inject(DETAIL, "onPreMovieFinish", opcode(PushScope), true, 1,
            join(call("hideModelAfterMappedPreMovie", 2, CallProperty, field(service),
                    field(member(DETAIL, "curPbi"))),
                List.of(op(IfFalse, 1), op(ReturnVoid))));
    }

    private void controls() throws Exception {
        int skin = member(CONTROLS, "skinId");
        int service = member(CONTROLS, "_skindataservice");
        var pet = field(member(CONTROLS, "pbi"));
        var mapped = visualId(service, pet);
        for (String name : List.of("updateTalkBtn", "isHuanDongSkin", "isCaiDanSkin", "isInterSkin")) {
            inject(CONTROLS, name, named(GetProperty, skin), true, name.equals("updateTalkBtn") ? 2 : 1, mapped);
        }
        inject(CONTROLS, "isHasPreMv", named(GetProperty, viewId), true, 1,
            decorate("preMovieSkinId", 3, field(service), pet));
        inject(CONTROLS, "updateTalkBtn", named(CallProperty, "hasTalk"), true, 1,
            decorate("showTalkButton", 3, field(service), pet));
        inject(CONTROLS, "updatePetInfo", opcode(PushScope), true, 1,
            join(callVoid("stopTalkOnSelection", 2, pet, List.of(op(GetLocal1))),
                callVoid("registerControls", 3, field(member(CONTROLS, "updatePreMvBtn")),
                    field(member(CONTROLS, "updateTalkBtn")), field(member(CONTROLS, "updateCaiDanBtn")))));
        // A mapped voice is local to this panel. Let the original handler
        // run its special unlock action with the original skin and arguments.
        inject(CONTROLS, "onClickYuyin", opcode(PushScope), true, 1,
            join(call("handleTalk", 3, CallProperty, field(service), pet, field(skin)),
                List.of(op(IfFalse, 1), op(ReturnVoid))));
    }

    private void head() throws Exception {
        inject(HEAD, "updateUI", named(CallPropVoid, member(HEAD, "loadYabiFightHead")), true, 1,
            callVoid("registerHead", 3, List.of(op(GetLocal0)), field(member(HEAD, "loadYabiFightHead")),
                field(member(HEAD, "_actPetLoader"))));
    }

    private List<AVM2Instruction> visualId(int service, List<AVM2Instruction> pet) {
        return visualId(field(service), pet);
    }

    private List<AVM2Instruction> visualId(List<AVM2Instruction> service, List<AVM2Instruction> pet) {
        return decorate("visualSkinId", 3, service, List.of(op(Swap)),
            join(pet, List.of(op(GetProperty, viewId), op(Swap))));
    }

    private List<AVM2Instruction> state(int service, List<AVM2Instruction> pet, boolean open) {
        return decorate("packageOpen", 4, field(service), pet, List.of(op(open ? PushTrue : PushFalse)));
    }

    @SafeVarargs
    private final List<AVM2Instruction> decorate(String name, int argc, List<AVM2Instruction>... args) {
        return call(name, argc, CallProperty, List.of(op(Swap)), join(args));
    }

    @SafeVarargs
    private final List<AVM2Instruction> callVoid(String name, int argc, List<AVM2Instruction>... args) {
        return call(name, argc, CallPropVoid, args);
    }

    @SafeVarargs
    private final List<AVM2Instruction> call(String name, int argc, int instruction, List<AVM2Instruction>... args) {
        int method = abc.constants.getPublicQnameId(name, !verify);
        return join(List.of(op(GetLex, helper)), join(args), List.of(op(instruction, method, argc)));
    }

    @SafeVarargs
    private final void inject(String owner, String name, Predicate<AVM2Instruction> anchor,
            boolean after, int expected, List<AVM2Instruction>... hooks) throws Exception {
        inject(body(owner, name), owner, name, anchor, after, expected, hooks);
    }

    @SafeVarargs
    private final void inject(MethodBody body, String owner, String name, Predicate<AVM2Instruction> anchor,
            boolean after, int expected, List<AVM2Instruction>... hooks) throws Exception {
        var code = body.getCode();
        var anchors = new ArrayList<Integer>();
        int count = 0;
        for (int i = 0; i < code.code.size(); i++) {
            if (!anchor.test(code.code.get(i))) continue;
            int at = i + (after ? 1 : 0);
            anchors.add(at);
            var hook = hooks.length == 1 ? hooks[0] : count < hooks.length ? hooks[count] : null;
            if (verify && hook != null) {
                int start = after ? at : at - hook.size();
                check(start >= 0 && start + hook.size() <= code.code.size(), owner, name);
                for (int j = 0; j < hook.size(); j++) {
                    check(Arrays.equals(code.code.get(start + j).getBytes(), hook.get(j).getBytes()), owner, name);
                }
                if (after) i += hook.size();
            }
            count++;
        }
        check(count == expected, owner, name);
        if (!verify) {
            for (int i = anchors.size() - 1; i >= 0; i--) {
                var hook = hooks.length == 1 ? hooks[0] : hooks[i];
                if (hook == null) continue;
                for (int j = hook.size() - 1; j >= 0; j--) {
                    var ins = hook.get(j);
                    // A single template may be inserted at several call sites;
                    // instructions need independent address/branch tracking.
                    code.insertInstruction(anchors.get(i), op(ins.definition.instructionCode, ins.operands), body);
                }
            }
            body.max_stack += 5;
            body.setCode(code);
        }
        code.checkValidOffsets(body);
    }

    private int member(String owner, String name) {
        return abc.instance_info.get(abc.findClassByName(owner)).instance_traits.traits.stream()
            .filter(t -> name.equals(abc.constants.getString(t.getName(abc).name_index)))
            .findFirst().orElseThrow(() -> new IllegalArgumentException(owner + "." + name)).name_index;
    }

    private MethodBody body(String owner, String name) {
        int index = member(owner, name);
        var traits = abc.instance_info.get(abc.findClassByName(owner)).instance_traits.traits;
        var method = (TraitMethodGetterSetter) traits.stream().filter(t -> t.name_index == index)
            .findFirst().orElseThrow();
        return abc.findBody(method.method_info);
    }

    private Predicate<AVM2Instruction> named(int code, String name) {
        return i -> opcode(code).test(i)
            && name.equals(abc.constants.getString(abc.constants.getMultiname(i.operands[0]).name_index));
    }

    private static Predicate<AVM2Instruction> named(int code, int index) {
        return i -> opcode(code).test(i) && i.operands[0] == index;
    }

    private static Predicate<AVM2Instruction> opcode(int code) {
        return i -> i.definition.instructionCode == code;
    }

    private static List<AVM2Instruction> field(int index) {
        if (index <= 0) throw new IllegalArgumentException("Missing visual hook property");
        return List.of(op(GetLocal0), op(GetProperty, index));
    }

    @SafeVarargs
    private static List<AVM2Instruction> join(List<AVM2Instruction>... parts) {
        var result = new ArrayList<AVM2Instruction>();
        for (var part : parts) result.addAll(part);
        return result;
    }

    private static AVM2Instruction op(int code, int... args) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[code], args);
    }

    private static void check(boolean found, String owner, String method) {
        if (!found) throw new IllegalArgumentException("Unrecognized visual skin hook layout in " + owner + "." + method);
    }
}
