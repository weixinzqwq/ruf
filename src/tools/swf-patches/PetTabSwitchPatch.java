import java.io.ByteArrayOutputStream;
import java.util.ArrayList;
import java.util.Arrays;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.InstanceInfo;
import com.jpexs.decompiler.flash.abc.types.MethodBody;
import com.jpexs.decompiler.flash.abc.types.traits.Trait;
import com.jpexs.decompiler.flash.abc.types.traits.TraitMethodGetterSetter;
import com.jpexs.decompiler.flash.abc.types.traits.TraitSlotConst;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

// Opt in only leaf pages whose detail snapshots are consumed while showing.
// The existing updateUI/removeUI calls from setTabInfoVisible remain the source of visibility.
class PetTabSwitchPatch {
    static final String PACKAGE = "mmo.petpackage.panel.inclusion.petdetailinfo.";
    static final String[] PAGES = {"PetDetailAbility", "PetStudyInfo", "PetItem", "GodCard"};
    static final int ACTIVE = 1, HIDDEN = 2, DISPOSED = 3; // 0 is an unmanaged controller.

    static class Code extends ByteArrayOutputStream {
        void op(int opcode, int... args) {
            writeBytes(new AVM2Instruction(0, AVM2Code.instructionSet[opcode], args).getBytes());
        }
        final int listener;
        Code(int listener) { this.listener = listener; }
        // The private bound Function is already kept on this instance for add/removeEventListener.
        // Use its dynamic properties so compiled subclasses keep every original instance slot index.
        void receiver() { op(GetLocal0); op(GetProperty, listener); }
        void read(int property) { receiver(); op(GetProperty, property); }
        void readMode(int property) { read(property); op(ConvertI); } // absent on unmanaged classes
        void mode(int property, int value) { receiver(); op(PushByte, value); op(SetProperty, property); }
        void clear(int property) { receiver(); op(PushNull); op(SetProperty, property); }
        // All new jumps are forward, within the prefix or to the old method.
        int branch(int opcode) { int pos = size(); op(opcode, 0); return pos; }
        void land(int pos) {
            int delta = size() - pos - 4;
            for (int i = 0; i < 3; i++) buf[pos + i + 1] = (byte)(delta >> (8 * i));
        }
    }

    static Trait trait(ABC abc, InstanceInfo owner, String name) {
        var matches = owner.instance_traits.traits.stream()
            .filter(t -> name.equals(abc.constants.getString(t.getName(abc).name_index))).toList();
        if (matches.size() != 1) throw new IllegalArgumentException("Unexpected trait: " + name);
        return matches.getFirst();
    }
    static MethodBody method(ABC abc, InstanceInfo owner, String name) {
        return abc.findBody(((TraitMethodGetterSetter)trait(abc, owner, name)).method_info);
    }
    static void insert(MethodBody body, int pos, Code insert) {
        byte[] old = body.getCodeBytes();
        if (body.exceptions.length != 0 || old[0] != (byte)GetLocal0 || old[1] != (byte)PushScope)
            throw new IllegalArgumentException("Unexpected scope or exception table in " + body.method_info);
        var result = new ByteArrayOutputStream();
        result.write(old, 0, pos);
        result.writeBytes(insert.toByteArray());
        result.write(old, pos, old.length - pos);
        body.max_stack = Math.max(body.max_stack, 3);
        body.setCodeBytes(result.toByteArray());
    }

    static void apply(ABC abc) {
        var c = abc.constants;
        var base = abc.instance_info.get(abc.findClassByName(PACKAGE + "PetDetailBase"));
        var panel = abc.instance_info.get(abc.findClassByName("mmo.petpackage.panel.PetPackagePanel"));
        var show = method(abc, panel, "setTabInfoVisible").getCode().code;
        var buildTabs = method(abc, panel, "initPars").getCode().code;
        var fields = new String[]{"mcsynthesis", "mcstudy", "mcitem", "mcgodcard"};
        var constructors = new ArrayList<MethodBody>();
        for (int i = 0; i < PAGES.length; i++) {
            var leaf = abc.instance_info.get(abc.findClassByName(PACKAGE + PAGES[i]));
            var field = (TraitSlotConst)trait(abc, panel, fields[i]);
            if (leaf.super_index != base.name_index || field.type_index != leaf.name_index
                || buildTabs.stream().noneMatch(op -> op.definition.instructionCode == GetProperty && op.operands[0] == field.name_index))
                throw new IllegalArgumentException("Unexpected tab owner " + PAGES[i]);
            var ctor = abc.findBody(leaf.iinit_index);
            var ops = ctor.getCode().code;
            if (ops.getLast().definition.instructionCode != ReturnVoid ||
                ops.stream().filter(op -> op.definition.instructionCode == ConstructSuper).count() != 1 ||
                ops.stream().anyMatch(op -> op.isBranch() || op.isJump()))
                throw new IllegalArgumentException("Unexpected tab constructor " + PAGES[i]);
            constructors.add(ctor);
        }
        for (String name : new String[]{"updateUI", "removeUI"}) {
            if (show.stream().noneMatch(op -> op.definition.instructionCode == CallPropVoid &&
                name.equals(c.getString(c.getMultiname(op.operands[0]).name_index))))
                throw new IllegalArgumentException("Missing tab lifecycle " + name);
        }
        var event = method(abc, base, "onPetDetailedCached");
        var ops = event.getCode().code;
        var eventOps = new int[]{GetLocal0, PushScope, GetLocal0, GetLex, GetProperty, GetLocal1,
            GetProperty, GetProperty, CallProperty, SetProperty, GetLocal0, CallPropVoid, ReturnVoid};
        if (!Arrays.equals(eventOps, ops.stream().mapToInt(op -> op.definition.instructionCode).toArray()) ||
            !c.getString(c.getMultiname(ops.get(8).operands[0]).name_index).equals("getPetDetailInfo"))
            throw new IllegalArgumentException("Unexpected detail lookup body");
        var remove = method(abc, base, "removeUI");
        if (!Arrays.equals(remove.getCodeBytes(), new byte[]{(byte)GetLocal0, (byte)PushScope, (byte)ReturnVoid}))
            throw new IllegalArgumentException("removeUI has its own cleanup");
        int listener = trait(abc, base, "onPetDetailedCached").name_index;
        if (c.getStringId("aolaDetailTabMode", false) > 0 || c.getStringId("aolaPendingDetailId", false) > 0)
            throw new IllegalArgumentException("Detail tab fields already exist");
        int mode = c.getPublicQnameId("aolaDetailTabMode", true);
        int pending = c.getPublicQnameId("aolaPendingDetailId", true);
        for (var ctor : constructors) {
            var optIn = new Code(listener);
            optIn.mode(mode, ACTIVE);
            insert(ctor, ctor.getCodeBytes().length - 1, optIn);
        }

        var hide = new Code(listener);
        hide.readMode(mode); hide.op(PushByte, ACTIVE); int keep = hide.branch(IfNe);
        hide.mode(mode, HIDDEN); hide.land(keep);
        insert(remove, 2, hide);

        var defer = new Code(listener);
        defer.readMode(mode); defer.op(PushByte, HIDDEN); int live = defer.branch(IfLt);
        defer.readMode(mode); defer.op(PushByte, DISPOSED); int done = defer.branch(IfEq);
        defer.receiver();
        for (int i = 5; i <= 7; i++) defer.writeBytes(ops.get(i).getBytes()); // event.params.id
        defer.op(ConvertI); defer.op(SetProperty, pending);
        defer.land(done); defer.op(ReturnVoid); defer.land(live);
        insert(event, 2, defer);

        var resume = new Code(listener);
        resume.readMode(mode); resume.op(PushByte, DISPOSED); int notDisposed = resume.branch(IfNe);
        resume.op(ReturnVoid); resume.land(notDisposed);
        resume.readMode(mode); resume.op(PushByte, HIDDEN); int ordinary = resume.branch(IfNe);
        resume.mode(mode, ACTIVE);
        resume.read(pending); resume.op(PushNull); int clean = resume.branch(IfEq);
        resume.read(pending); resume.op(SetLocal1); resume.clear(pending);
        for (int i = 2; i <= 4; i++) resume.writeBytes(ops.get(i).getBytes()); // this, map.instance
        resume.op(GetLocal1); resume.writeBytes(ops.get(8).getBytes()); resume.writeBytes(ops.get(9).getBytes());
        resume.land(clean); resume.land(ordinary);
        var update = method(abc, base, "updateUI");
        if (update.max_regs != 1) throw new IllegalArgumentException("Unexpected updateUI locals");
        update.max_regs = 2;
        insert(update, 2, resume);

        // Async initialization still marks canUpdateUI and installs its resources.
        // A callback completing after hide/dispose must not paint the obsolete view.
        var guard = new Code(listener);
        guard.readMode(mode); guard.op(PushByte, HIDDEN); int active = guard.branch(IfLt);
        guard.op(ReturnVoid); guard.land(active);
        insert(method(abc, base, "tryRefreshUI"), 2, guard);

        var finish = new Code(listener);
        finish.readMode(mode); int unmanaged = finish.branch(IfFalse);
        finish.mode(mode, DISPOSED); finish.clear(pending); finish.land(unmanaged);
        insert(method(abc, base, "dispose"), 2, finish);
    }
}
