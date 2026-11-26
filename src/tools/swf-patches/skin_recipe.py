"""Emit the small Rust edit table for the reviewed offline skin candidate."""
from difflib import SequenceMatcher


def recipe_edits(original, candidate):
    edits = []
    left = right = 0
    while left < len(original) and right < len(candidate):
        if original[left] == candidate[right]:
            left += 1
            right += 1
            continue
        # Limit the search to the next local edit. The movie's artwork and all
        # unchanged large ranges are scanned linearly.
        a = original[left:left + 512]
        b = candidate[right:right + 512]
        matcher = SequenceMatcher(None, a, b, autojunk=False)
        anchor = next((block for block in matcher.get_matching_blocks() if block.size >= 16), None)
        if anchor is None:
            raise ValueError("No local resynchronization point for the reviewed skin recipe")
        for kind, i, end_i, j, end_j in matcher.get_opcodes():
            if i >= anchor.a and j >= anchor.b:
                break
            if kind != "equal":
                edits.append((left + i, a[i:end_i], b[j:end_j]))
        left += anchor.a
        right += anchor.b
    if left < len(original) or right < len(candidate):
        edits.append((left, original[left:], candidate[right:]))
    return edits


def rust_bytes(value):
    if len(value) > 80:
        raise ValueError("Edit exceeds the expected limit for this skin patch")
    if len(value) <= 12:
        return "&[" + ", ".join(f"0x{b:02x}" for b in value) + "]"
    lines = [", ".join(f"0x{b:02x}" for b in value[i:i + 12]) for i in range(0, len(value), 12)]
    return "&[\n        " + ",\n        ".join(lines) + ",\n    ]"


def rust_table(original, candidate):
    edits = recipe_edits(original, candidate)
    if sum(len(old) + len(new) for _, old, new in edits) > 512:
        raise ValueError("Unexpectedly large delta for the archived skin candidate")
    lines = ["#[rustfmt::skip]", "const EDITS: &[(usize, &[u8], &[u8])] = &["]
    # Original body offsets from the audited 13-method rebuild. Keep the byte
    # ranges in their actual file order so the embedded table can be reviewed.
    owners = {
        1980781: "DoABC length",
        1980934: "ABC string pool",
        2046380: "ABC multiname pool",
        2097202: "PackageSkinManager.joinNormals",
        2097243: "PackageSkinManager.joinSimple",
        2097393: "PackageSkinManager.init",
        2097515: "PackageSkinManager.changeClassify",
        2097653: "PackageSkinManager.dispose",
        2098060: "Style.dispose",
        2098524: "MotionBase.appear",
        2098635: "MotionBase.dispose",
        2099055: "MotionLeft.disappear",
        2099196: "MotionDown.disappear",
        2099342: "MotionRight.disappear",
        2099483: "MotionUp.disappear",
        2099625: "MotionNormal.disappear",
    }
    for offset, old, new in edits:
        if offset in owners:
            lines.append(f"    // {owners[offset]}")
        lines.append(f"    ({offset}, {rust_bytes(old)}, {rust_bytes(new)}),")
    return "\n".join([*lines, "];", ""])
