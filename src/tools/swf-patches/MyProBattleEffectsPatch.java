import java.io.ByteArrayOutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.HashMap;
import java.util.Map;
import com.jpexs.decompiler.flash.SWF;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.ABCInputStream;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.Namespace;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import com.jpexs.helpers.MemoryInputStream;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

/** Inserts presentation guards at empty-stack boundaries in the supported battle ABC. */
class MyProBattleEffectsPatch {
    static final String BASE = "mmo.petfightnew.common.";
    static final String CTRL = "fightcontroller.";
    final ABC abc;
    final int helper;
    String owner;
    String methodName;
    MethodBody body;
    final Map<Integer, byte[]> prefixes = new HashMap<>();
    final Map<Integer, byte[]> checked = new HashMap<>();

    MyProBattleEffectsPatch(ABC abc) {
        this.abc = abc;
        helper = abc.constants.getQnameId("MyProBattleEffects", Namespace.KIND_PACKAGE, "aola.tools", true);
    }

    int pub(String name) { return abc.constants.getPublicQnameId(name, true); }

    int member(String name) {
        var traits = abc.instance_info.get(abc.findClassByName(BASE + owner)).instance_traits.traits;
        var matches = traits.stream().filter(t -> name.equals(abc.constants.getString(t.getName(abc).name_index))).toList();
        if (matches.size() != 1) throw new IllegalStateException(owner + "." + name + " is ambiguous");
        return matches.getFirst().name_index;
    }

    void method(String owner, String name) {
        this.owner = owner;
        this.methodName = name;
        var traits = abc.instance_info.get(abc.findClassByName(BASE + owner)).instance_traits.traits;
        int key = member(name);
        for (var t : traits) {
            if (t.name_index == key && t instanceof TraitMethodGetterSetter m) {
                body = abc.findBody(m.method_info);
                if (body == null || body.exceptions.length != 0)
                    throw new IllegalStateException(BASE + owner + "." + name + " expected a body without exception regions");
                return;
            }
        }
        throw new IllegalStateException("Unsupported method " + owner + "." + name);
    }

    static byte[] op(int code, int... operands) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[code], operands).getBytes();
    }

    static byte[] seq(byte[]... items) {
        var out = new ByteArrayOutputStream();
        for (var item : items) out.writeBytes(item);
        return out.toByteArray();
    }

    static byte[] whenTrue(byte[] action) { return seq(op(IfFalse, action.length), action); }
    static final byte[] SELF = op(GetLocal0), END = op(ReturnVoid);
    static byte[] slot(int n) { return seq(op(GetScopeObject, 1), op(GetSlot, n)); }
    byte[] field(String name) { return seq(SELF, op(GetProperty, member(name))); }
    byte[] invoke(String name, int n) { return op(CallPropVoid, member(name), n); }

    AVM2Instruction at(int addr, int opcode, int... args) {
        var ins = body.getCode().code.stream().filter(i -> i.getAddress() == addr).findFirst().orElse(null);
        if (ins == null || ins.definition != AVM2Code.instructionSet[opcode] || (args.length > 0 && !Arrays.equals(args, ins.operands)))
            throw new IllegalStateException(BASE + owner + "." + methodName + " expected " +
                    AVM2Code.instructionSet[opcode].instructionName + " at " + addr);
        return ins;
    }

    void guard(int addr, String flag, byte[] action) {
        boolean multi = flag.equals("multiple");
        var bytes = seq(op(GetLex, helper), multi ? field("_hpInc") : new byte[0],
                op(CallProperty, pub(flag), multi ? 1 : 0), whenTrue(action));
        if (prefixes.put(addr, bytes) != null) throw new IllegalStateException("Duplicate guard");
    }

    // An original branch entering a guarded block must evaluate its guard too. Relocate
    // to the start of the prefix, and independently verify every resulting jump edge.
    void finish() throws Exception {
        var original = body.getCode().code;
        var before = new HashMap<Long, Integer>();
        int length = 0;
        for (var i : original) {
            before.put(i.getAddress(), length);
            length += prefixes.getOrDefault((int)i.getAddress(), new byte[0]).length + i.getBytesLength();
        }
        var out = new ByteArrayOutputStream(length);
        for (var i : original) {
            var prefix = prefixes.remove((int)i.getAddress());
            if (prefix != null) out.writeBytes(prefix);
            if (i.definition == AVM2Code.instructionSet[LookupSwitch]) throw new IllegalStateException("Unexpected switch");
            if (i.isJump() || i.isBranch()) {
                int delta = before.get(i.getTargetAddress()) - (out.size() + i.getBytesLength());
                out.writeBytes(op(i.definition.instructionCode, delta));
            } else out.writeBytes(i.getBytes());
        }
        if (!prefixes.isEmpty()) throw new IllegalStateException("Guard address missing");
        body.setCodeBytes(out.toByteArray());
        body.max_stack = Math.max(body.max_stack, 5);
        var rewritten = body.getCode();
        rewritten.checkValidOffsets(body);
        for (int j = 0; j < original.size(); j++) {
            var i = original.get(j);
            long next = j + 1 == original.size() ? out.size() : before.get(original.get(j + 1).getAddress());
            var updated = rewritten.code.get(rewritten.adr2pos(next - i.getBytesLength()));
            if (i.definition != updated.definition || (i.isJump() || i.isBranch()
                    ? updated.getTargetAddress() != before.get(i.getTargetAddress())
                    : !Arrays.equals(updated.getBytes(), i.getBytes()))) throw new IllegalStateException("Branch relocation failed");
        }
        checked.put(body.method_info, out.toByteArray());
        System.out.println(owner + " #" + body.method_info + " verified");
    }

    void numbers() throws Exception {
        String[] names = {"showMultipleHit", "showIncMovie", "showReallyHit", "showShieldHit", "showReallyShieldHit",
                "playStrongMovie", "setNormalMovie", "playAllHitMovie"};
        for (int i = 0; i < names.length; i++) {
            method("SkillResultMovieHandler", names[i]);
            at(5, PushScope);
            guard(6, i == 0 ? "multiple" : i == 1 ? "healing" : "damage", END);
            finish();
        }
    }

    void pets() throws Exception {
        method("MCPetContainer", "onResPrepared");
        at(0x78, CallPropVoid, member("setMiracleLight"), 0);
        guard(0x7c, "skip", seq(SELF, SELF, op(GetProperty, pub("locaId")), invoke("dispatchGotPet", 1), END));
        finish();
        method("MCPetContainer", "onStartCriEff");
        at(1, PushScope);
        guard(2, "skip", END);
        finish();
        method(CTRL + "SwitchPm", "showSwitchMc");
        at(0x11, IfFalse);
        at(0x15, GetLocal2); // A missing pet has no switch animation or completion.
        guard(0x15, "skip", seq(SELF, op(GetLocal1), invoke("onSwitchEnd", 1), END));
        finish();
        method(CTRL + "ChangePetView", "onPetChange");
        int accessor = at(0x55, SetProperty).operands[0];
        at(0x6a, CallPropVoid, pub("addChildAt"), 2);
        guard(0x6e, "skip", seq(field("_petContainer"), op(PushByte, 1), op(SetProperty, pub("scaleX")),
                field("_petContainer"), op(PushByte, 1), op(SetProperty, pub("scaleY")), END));
        finish();
        method(CTRL + "ChangePetView", "onGotPet");
        at(0x21, ReturnVoid);
        at(0x23, GetProperty, member("_curPet"));
        guard(0x22, "skip", seq(field("fightPanel"), op(GetProperty, pub("petMCList")), field("_locaId"),
                op(GetProperty, accessor), op(SetLocal2), SELF, op(GetLocal2), op(GetProperty, pub("parent")),
                op(GetLocal2), op(CallProperty, pub("getChildIndex"), 1), op(GetLocal2), invoke("onPetChange", 2),
                SELF, invoke("onEffectMoviePlayed", 0), END));
        finish();
        method(CTRL + "BattlePmDying", "showPetDieEffect");
        at(0x34, SetSlot, 4);
        guard(0x36, "skip", seq(slot(4), whenTrue(seq(slot(4), op(CallPropVoid, pub("disposeMouseResponder"), 0),
                slot(4), op(PushFalse), op(SetProperty, pub("visible")))),
                slot(2), op(GetGlobalScope), op(Call, 0), op(Pop), END));
        finish();
    }

    byte[] skillDataAndMovie(byte[] params) {
        return seq(SELF, params, invoke("handleDatas", 1), SELF, params, invoke("handleMovie", 1), END);
    }

    byte[] updatePanel(String action, String value) {
        return seq(field("fightPanel"), field(value), op(CallPropVoid, pub(action), 1));
    }

    void skills() throws Exception {
        String use = CTRL + "UseSkill";
        method(use, "handleUseSkill");
        at(0x181, ReturnVoid); at(0x182, GetLocal0);
        at(0x183, FindPropertyStrict, abc.constants.getQnameId("AoYiMovieHelper", Namespace.KIND_PACKAGE,
                BASE + "fightcontroller", false));
        guard(0x182, "skip", seq(SELF, slot(5), slot(1), invoke("handleAoYiSkill", 2), END));
        at(0x1ac, ReturnVoid); at(0x1ad, GetLocal0);
        at(0x1b2, CallProperty, member("isNeedToHandleWangzheReal"), 1);
        guard(0x1ad, "skip", skillDataAndMovie(slot(1)));
        finish();
        method(use, "handleAnnalFeeEight");
        at(0x28, SetSlot, 2);
        guard(0x2a, "skip", skillDataAndMovie(op(GetLocal2)));
        finish();
        method(use, "handleMovie");
        at(0x1b3, ReturnVoid); at(0x1b4, GetScopeObject, 1);
        int manager = abc.constants.getQnameId("FightFrameRateManager", Namespace.KIND_PACKAGE, BASE + "manager", false);
        if (manager == 0) throw new IllegalStateException("Frame rate manager missing");
        guard(0x1b4, "skip", seq(op(GetLex, manager), op(GetProperty, pub("instance")),
                op(CallPropVoid, pub("correctFrameRate"), 0), SELF, invoke("refreshInfos", 0),
                updatePanel("setBS", "skillBS"), updatePanel("setBackGroundUp", "_bus"),
                updatePanel("setPetUp", "_skillParams"), SELF, invoke("handleSkillEnd", 0), END));
        finish();
        method(use, "getMuiltHitDecHpDelayFrame");
        at(1, PushScope);
        guard(2, "skip", seq(op(PushByte, 1), op(ReturnValue)));
        finish();
    }

    void queue() throws Exception {
        method(CTRL + "BattleNextTurn", "checkPlayMovie");
        at(1, PushScope);
        guard(2, "skip", seq(SELF, invoke("handle", 0), END));
        finish();
        method("FightController", "startHandle");
        at(0x1c, GetLex, abc.constants.getQnameId("EnterFrameManager", Namespace.KIND_PACKAGE, "mmo.common.event", false));
        guard(0x1c, "skip", seq(SELF, op(CallPropVoid, pub("handle"), 0), END));
        finish();
    }

    public static void main(String[] args) throws Exception {
        ABC abc;
        try (var stream = Files.newInputStream(Path.of(args[0]))) {
            var games = new SWF(stream, false).getAbcList().stream().map(t -> t.getABC())
                    .filter(a -> a.findClassByName(BASE + "SkillResultMovieHandler") >= 0).toList();
            if (games.size() != 1) throw new IllegalStateException("Expected one battle ABC");
            abc = games.getFirst();
        }
        try (var out = Files.newOutputStream(Path.of(args[1]))) { abc.saveToStream(out); }
        var originals = new HashMap<Integer, byte[]>();
        for (var b : abc.bodies) originals.put(b.method_info, b.getCodeBytes().clone());
        var patch = new MyProBattleEffectsPatch(abc);
        patch.numbers(); patch.pets(); patch.skills(); patch.queue();
        try (var out = Files.newOutputStream(Path.of(args[2]))) { abc.saveToStream(out); }
        var stored = new ABC(new ABCInputStream(new MemoryInputStream(Files.readAllBytes(Path.of(args[2])))),
                abc.parentTag.getSwf(), abc.parentTag);
        if (patch.checked.size() != 20 || stored.bodies.size() != originals.size())
            throw new IllegalStateException("Unexpected number of changed methods");
        for (var b : stored.bodies) {
            var expected = patch.checked.getOrDefault(b.method_info, originals.get(b.method_info));
            if (!Arrays.equals(b.getCodeBytes(), expected)) throw new IllegalStateException("Serialized method #" + b.method_info + " mismatch");
        }
        System.out.println("Re-read 20 guarded methods and " + (originals.size() - 20) + " untouched methods");
    }
}
