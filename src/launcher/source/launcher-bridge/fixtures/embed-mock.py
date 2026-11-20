"""Insert the offline mock class before the real, unmodified bridge DoABC."""
from pathlib import Path
import struct

root = Path(__file__).resolve().parent
data = (root.parent / "variants/start-launcher.fws.swf").read_bytes()
position = 8 + (5 + (data[8] >> 3) * 4 + 7) // 8 + 4
while True:
    tag_start = position
    tag = struct.unpack_from("<H", data, position)[0]
    position += 2
    code, size = tag >> 6, tag & 63
    if size == 63:
        size = struct.unpack_from("<I", data, position)[0]
        position += 4
    if code == 82:
        break
    position += size
abc = (root / "ActClientSender.abc").read_bytes()
payload = struct.pack("<I", 1) + b"offline-mock\0" + abc
mock_tag = struct.pack("<HI", (82 << 6) | 63, len(payload)) + payload
result = data[:tag_start] + mock_tag + data[tag_start:]
result = result[:4] + struct.pack("<I", len(result)) + result[8:]
(root / "start-with-mock.swf").write_bytes(result)
assert result[tag_start + len(mock_tag):] == data[tag_start:]
print(f"Offline-only mock fixture: {len(result)} bytes; real bridge DoABC unchanged")
