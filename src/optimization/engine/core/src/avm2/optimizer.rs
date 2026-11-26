mod blocks;
mod dce;
mod nop_remover;
mod peephole;
mod type_aware;

use crate::avm2::activation::Activation;
use crate::avm2::error::Error;
use crate::avm2::method::{Method, ResolvedParamConfig};
use crate::avm2::op::Op;
use crate::avm2::verify::Exception;

use std::cell::Cell;
use std::collections::HashSet;

/// Run all the optimizer passes on the given code. This method should be
/// run regardless of whether or not the "disable AVM2 optimizer" player option
/// is on. It will not perform observable optimizations if the "disable
/// AVM2 optimizer" player option is off.
pub fn optimize<'gc>(
    activation: &mut Activation<'_, 'gc>,
    method: Method<'gc>,
    code: &mut Vec<Op<'gc>>,
    method_exceptions: &mut [Exception<'gc>],
    resolved_parameters: &[ResolvedParamConfig<'gc>],
    mut jump_targets: HashSet<usize>,
) -> Result<(), Error<'gc>> {
    let code_slice = Cell::from_mut(code.as_mut_slice());
    let code_slice = code_slice.as_slice_of_cells();

    let sets_local_0 = determine_sets_local_0(code_slice);
    // Save the four source call sites before specialization erases their property names.
    let switch_sites = (method.swf_operation() == super::swf_operation::Operation::SwitchPet)
        .then(|| pet_switch_call_sites(code_slice));

    // We run the preprocess peephole before assembling blocks because it removes
    // zero-length jumps, which usually reduces the number of blocks in obfuscated code.
    peephole::preprocess_peephole(code_slice);

    type_aware::type_aware_optimize(
        activation,
        method,
        code_slice,
        method_exceptions,
        resolved_parameters,
        &mut jump_targets,
        sets_local_0,
    )?;

    peephole::postprocess_peephole(code_slice, &jump_targets);

    dce::eliminate_dead_code(code_slice, &jump_targets);

    if let Some(sites) = switch_sites {
        if let Some(sites) = sites.filter(|sites| sites.iter().all(|&i| matches!(code_slice[i].get(),
            Op::CallPropVoid { .. } | Op::CallMethod { .. } | Op::CallNative { .. }
            | Op::ConstructProp { .. } | Op::ConstructSlot { .. }))) {
            for (phase, i) in sites.into_iter().enumerate() {
                let original = gc_arena::Gc::new(activation.gc(), code_slice[i].get());
                code_slice[i].set(Op::TimedPetSwitch { phase: phase as u8, original });
            }
            tracing::info!(target: "aola_swf_operation", operation = "pet-switch-timing-bound");
        } else {
            tracing::warn!(target: "aola_swf_operation", operation = "pet-switch-timing-unbound");
        }
    }
    nop_remover::remove_nops(code, method_exceptions);

    Ok(())
}

fn pet_switch_call_sites(code: &[Cell<Op<'_>>]) -> Option<[usize; 4]> {
    let mut sites = [0; 4];
    let mut next = 0;
    for (i, op) in code.iter().enumerate() {
        let (name, args, constructor) = match op.get() {
            Op::CallPropVoid { multiname, num_args } => (multiname.local_name(), num_args, false),
            Op::ConstructProp { multiname, num_args } => (multiname.local_name(), num_args, true),
            _ => continue,
        };
        let Some(name) = name else { continue };
        let phase = match (name.to_utf8_lossy().as_ref(), args, constructor) {
            ("dispose", 0, false) => 0,
            ("stopAndRemove", 1, false) => 1,
            ("MCPetContainer", 4, true) => 2,
            ("startLoad", 0, false) => 3,
            _ => continue,
        };
        if phase != next { return None }
        sites[phase] = i;
        next += 1;
    }
    (next == sites.len()).then_some(sites)
}

fn determine_sets_local_0<'gc>(ops: &[Cell<Op<'gc>>]) -> bool {
    for op in ops {
        match op.get() {
            Op::SetLocal { index }
            | Op::Kill { index }
            | Op::DecLocal { index }
            | Op::DecLocalI { index }
            | Op::IncLocal { index }
            | Op::IncLocalI { index }
                if index == 0 =>
            {
                return true;
            }
            Op::HasNext2 {
                object_register,
                index_register,
            } if object_register == 0 || index_register == 0 => {
                return true;
            }
            _ => {}
        }
    }

    false
}
