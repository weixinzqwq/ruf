import java.util.List;
import com.jpexs.decompiler.flash.abc.ABC;
import com.jpexs.decompiler.flash.abc.avm2.AVM2Code;
import com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instruction;
import com.jpexs.decompiler.flash.abc.types.ValueKind;
import static com.jpexs.decompiler.flash.abc.avm2.instructions.AVM2Instructions.*;

class PetActiveTabSwitchPatch {
    static final String PANEL = "mmo.petpackage.panel.PetPackagePanel";
    static final String DETAIL = "mmo.petpackage.panel.inclusion.petdetailinfo.";
    static final String HELPER = "aolaUpdateSkillOnDetailBroadcast";

    static AVM2Instruction op(int opcode, int... args) {
        return new AVM2Instruction(0, AVM2Code.instructionSet[opcode], args);
    }

    static void target(AVM2Instruction jump, AVM2Instruction landing) {
        jump.operands[0] = (int)(landing.getAddress() - jump.getAddress() - jump.getBytesLength());
    }

    static void apply(ABC abc) throws Exception {
        var c = abc.constants;
        var panel = abc.instance_info.get(abc.findClassByName(PANEL));
        int skillIndex = abc.findClassByName(DETAIL + "PetDetailSkill");
        var skill = abc.instance_info.get(skillIndex);
        var base = abc.instance_info.get(abc.findClassByName(DETAIL + "PetDetailBase"));
        var skillField = PetTabSwitchPatch.trait(abc, panel, "mcskill");
        var display = PetTabSwitchPatch.method(abc, panel, "setTabInfoVisible");
        var event = PetTabSwitchPatch.method(abc, panel, "onCurrentPetUpdate");
        var ready = PetTabSwitchPatch.trait(abc, base, "canUpdateUI").name_index;
        var signature = abc.method_info.get(display.method_info);
        if (skill.super_index != base.name_index || c.getStringId(HELPER, false) > 0
                || signature.param_types.length != 0 || signature.flags != 0
                || display.exceptions.length != 0 || event.exceptions.length != 0)
            throw new IllegalArgumentException("Unexpected active skill tab layout");

        // Only the canonical PET_DETAIL_UPDATE listener opts in. Initialization,
        // user tab changes and other callers use the original default.
        var eventCode = event.getCode();
        var displayName = PetTabSwitchPatch.trait(abc, panel, "setTabInfoVisible").name_index;
        var calls = eventCode.code.stream().filter(i -> i.definition.instructionCode == CallPropVoid
                && i.operands[0] == displayName && i.operands[1] == 0).toList();
        if (calls.size() != 1) throw new IllegalArgumentException("Expected one detail-event call");
        var call = calls.getFirst();
        eventCode.insertInstruction(eventCode.code.indexOf(call), op(PushTrue), event);
        call.operands[1] = 1;
        event.max_stack++;
        eventCode.checkValidOffsets(event);
        event.setCode(eventCode);

        var code = display.getCode();
        var draws = code.code.stream().filter(i -> i.definition.instructionCode == CallPropVoid
                && c.getString(c.getMultiname(i.operands[0]).name_index).equals("updateUI")
                && i.operands[1] == 0).toList();
        if (draws.size() != 1 || code.code.get(1).definition.instructionCode != PushScope
                || code.code.stream().filter(i -> i.definition.instructionCode == GetLocal1
                        || i.definition.instructionCode == SetLocal1).findFirst().orElseThrow()
                        .definition.instructionCode != SetLocal1)
            throw new IllegalArgumentException("Unexpected selected-tab update");
        signature.param_types = new int[]{c.getPublicQnameId("Boolean", true)};
        signature.setFlagHas_optional();
        signature.optional = new ValueKind[]{new ValueKind(0, ValueKind.CONSTANT_False)};
        int detailBroadcast = display.max_regs++;
        code.insertInstruction(2, op(GetLocal1), display);
        code.insertInstruction(3, op(SetLocal, detailBroadcast), display);

        // The page's own detail listener still reads and paints the new pdi in
        // either listener order. Keep the normal update for first show, switching
        // back to the page, and outstanding asynchronous service initialization.
        var helper = abc.addMethod(skillIndex, HELPER, false);
        abc.method_info.get(helper.method_info).ret_type = c.getPublicQnameId("void", true);
        var body = abc.findBody(helper.method_info);
        var template = PetTabSwitchPatch.method(abc, skill, "refreshUI");
        body.init_scope_depth = template.init_scope_depth;
        body.max_scope_depth = template.max_scope_depth;
        body.max_stack = 1;
        body.max_regs = 1;
        var guard = new AVM2Code();
        var hidden = op(IfFalse, 0);
        var alreadyShowing = op(IfTrue, 0);
        var update = op(GetLocal0);
        var finish = op(ReturnVoid);
        guard.code.addAll(List.of(op(GetLocal0), op(PushScope),
                op(GetLocal0), op(GetProperty, c.getPublicQnameId("visible", false)), hidden,
                op(GetLocal0), op(GetProperty, ready), alreadyShowing,
                update, op(CallPropVoid, PetTabSwitchPatch.trait(abc, base, "updateUI").name_index, 0), finish));
        guard.markOffsets();
        target(hidden, update);
        target(alreadyShowing, finish);
        guard.checkValidOffsets(body);
        body.setCode(guard);

        var normal = draws.getFirst();
        int pos = code.code.indexOf(normal);
        var after = code.code.get(pos + 1);
        var ordinary = op(IfFalse, 0);
        var otherTab = op(IfStrictNe, 0);
        var done = op(Jump, 0);
        for (var instruction : List.of(op(GetLocal, detailBroadcast), ordinary, op(Dup), op(GetLocal0),
                op(GetProperty, skillField.name_index), otherTab, op(Coerce, skill.name_index),
                op(CallPropVoid, helper.name_index, 0), done)) {
            code.insertInstruction(pos++, instruction, display);
        }
        target(ordinary, normal);
        target(otherTab, normal);
        target(done, after);
        display.max_stack = Math.max(display.max_stack, 3);
        code.checkValidOffsets(display);
        display.setCode(code);
    }
}
