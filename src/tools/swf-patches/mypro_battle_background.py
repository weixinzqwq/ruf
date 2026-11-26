"""Attach the versioned, local JPEG overlay to the production battle background layer."""
import shutil
import struct
import subprocess
from pathlib import Path

from observe_skin_motion import ASC, ENGINE, JAVA, ROOT, iter_tags


HERE = Path(__file__).resolve().parent


def apply_background(input_fws: bytes, work: Path) -> bytes:
    work = Path(work) / "background"
    work.mkdir(parents=True, exist_ok=True)
    if input_fws[:3] != b"FWS" or struct.unpack_from("<I", input_fws, 4)[0] != len(input_fws):
        raise ValueError("Expected complete uncompressed battle SWF")
    source, old, new = [work / name for name in ("source.swf", "old.abc", "patched.abc")]
    source.write_bytes(input_fws)
    runner = [JAVA, "-cp", str(ROOT/"toolchain/ffdec/lib/*"), HERE/"MyProBattleBackgroundPatch.java"]
    subprocess.run([*runner, source, old, new], check=True)
    original_abc = old.read_bytes()
    (pos, payload, serialized), = [(pos,payload,serialized) for kind,pos,payload,serialized in iter_tags(input_fws[8:])
                                     if kind == 82 and payload[payload.index(0,4)+1:] == original_abc]
    header = payload[:payload.index(0,4)+1]
    globals_dir = ENGINE/"core/src/avm2/globals"
    as_file = work/"MyProBattleBackground.as"
    shutil.copyfile(HERE/as_file.name, as_file)
    playerglobal = work/"playerglobal.abc"
    subprocess.run([JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize", "-builtin", "-apiversioning",
                    "-outdir", work, "-out", "playerglobal", globals_dir/"Toplevel.as", globals_dir/"globals.as"], check=True)
    subprocess.run([JAVA, "-jar", ASC, "-AS3", "-import", playerglobal, as_file], check=True)
    helper = struct.pack("<I", 1) + b"aola.tools.MyProBattleBackground\x00" + as_file.with_suffix(".abc").read_bytes()
    def tag(data):
        return struct.pack("<HI", (82 << 6) | 63, len(data)) + data
    start = pos + 8
    body = input_fws[8:start] + tag(helper) + tag(header+new.read_bytes()) + input_fws[start+len(serialized):]
    result = input_fws[:4] + struct.pack("<I", len(body)+8) + body
    candidate = work/"result.swf"
    candidate.write_bytes(result)
    subprocess.run([*runner, "--verify", source, candidate], check=True)
    return result
