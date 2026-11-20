"""Compare the original/candidate SWF tags without changing either file."""
import json
import struct
import sys
from pathlib import Path

root = Path(__file__).resolve().parent

def tags(path):
    data = path.read_bytes()
    assert data[:3] == b"FWS"
    assert struct.unpack_from("<I", data, 4)[0] == len(data)
    rect_length = (5 + (data[8] >> 3) * 4 + 7) // 8
    position = 8 + rect_length + 4
    header = data[8:position]
    result = []
    while position < len(data):
        value = struct.unpack_from("<H", data, position)[0]
        position += 2
        code, length = value >> 6, value & 63
        if length == 63:
            length = struct.unpack_from("<I", data, position)[0]
            position += 4
        payload = data[position:position + length]
        assert len(payload) == length
        result.append((code, payload))
        position += length
    assert position == len(data)
    return header, result

original_header, original = tags(root / "source/start-original.fws.swf")
candidate_header, candidate = tags(Path(sys.argv[1]) if len(sys.argv) > 1 else root / "variants/start-launcher.fws.swf")
assert original_header == candidate_header
assert len(original) == len(candidate)
changed = [i for i, (a, b) in enumerate(zip(original, candidate)) if a != b]
assert changed and all(original[i][0] == candidate[i][0] == 82 for i in changed)
report = {"frame_header_equal": True, "tag_count": len(original),
          "changed_tag_indices": changed, "only_DoABC_changed": True,
          "all_non_code_tags_equal": True}
(root / "metadata/tag-comparison.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
print(json.dumps(report))
