"""Describe the exact allowed overlay on an already optimized, uncompressed game movie."""
from pathlib import Path
from observe_skin_motion import iter_tags
from skin_recipe import recipe_edits

RECIPES = Path(__file__).resolve().parents[2] / "optimization/engine/frontend-utils/src/backends/navigator"


def contract(baseline, candidate):
    def tags(movie):
        return list(iter_tags(movie[8:]))

    base_tags = tags(baseline)
    changes = []
    old = 0
    first_tag = base_tags[0][1] + 8
    if baseline[:first_tag] != candidate[:first_tag]:
        changes.append((0,baseline[:first_tag],candidate[:first_tag]))
    for kind, offset, payload, raw in tags(candidate):
        old_kind, old_offset, old_payload, old_raw = base_tags[old]
        at = old_offset + 8
        if kind == 82 and payload[4:payload.index(0, 4)] in (
            b"aola/tools/MyProBackpackMorph", b"aola.tools.MyProLineup",
            b"aola.tools.MyProBattleEffects", b"aola.tools.MyProBattleBackground", b"aola.tools.MyProNativeAuto"
        ):
            changes.append((at, b"", raw))
            continue
        if kind != old_kind:
            raise ValueError("Unexpected tag order in model overlay")
        old += 1
        if raw != old_raw:
            if kind != 82 or old_payload[4:old_payload.index(0, 4)] != payload[4:payload.index(0, 4)]:
                raise ValueError("Unexpected change outside existing game ABC")
            changes.extend((at + pos, before, after) for pos, before, after in recipe_edits(old_raw, raw))
    if old != len(base_tags):
        raise ValueError("Missing original SWF tag")
    rebuilt = bytearray()
    cursor = 0
    for pos, before, after in changes:
        if pos < cursor or baseline[pos:pos+len(before)] != before:
            raise ValueError("Invalid overlay ranges")
        rebuilt.extend(baseline[cursor:pos]); rebuilt.extend(after); cursor=pos+len(before)
    rebuilt.extend(baseline[cursor:])
    if rebuilt != candidate:
        raise ValueError("Overlay did not reproduce candidate")
    return changes


def model_table(baseline, candidate):
    changes=contract(baseline,candidate)
    lines=["// Generated from the checked model overlay; offsets refer to the optimized FWS.",
           "#[rustfmt::skip]", "pub(super) const EDITS: &[(usize, usize, &[u8])] = &["]
    for position,before,after in changes:
        literal=",".join(str(b) for b in after)
        lines.append(f"    ({position}, {len(before)}, &[{literal}]),")
    return "\n".join([*lines,"];", ""])


def verify_recipe(baseline, candidate, name, emit):
    text=model_table(baseline,candidate)
    path=RECIPES / f"aola_mypro_{name}.rs"
    if emit:
        path.write_text(text,encoding="ascii")
    elif text!=path.read_text(encoding="ascii"):
        raise ValueError("Model overlay differs from the reviewed profile")
