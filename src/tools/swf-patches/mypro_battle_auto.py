"""Add a read/set bridge to the existing Flash auto button in a (possibly multi-ABC) battle movie."""
import os
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

from observe_skin_motion import ASC, ENGINE, JAVA, ROOT, iter_tags, unpack_swf

HERE = Path(__file__).resolve().parent
TAG_NAME = b"aola.tools.MyProNativeAuto\0"


def abc_tag(payload):
    return struct.pack("<HI", (82 << 6) | 63, len(payload)) + payload


def apply_auto(input_fws: bytes, work: Path) -> bytes:
    if input_fws[:3] != b"FWS":
        raise ValueError("Native auto overlay expects uncompressed FWS")
    body = unpack_swf(input_fws)
    tags = [(offset, payload, raw) for kind, offset, payload, raw in iter_tags(body) if kind == 82]
    if any(payload[4:].startswith(TAG_NAME) for _, payload, _ in tags):
        raise ValueError("Native auto bridge already present")
    work.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="native-auto-", dir=work)).resolve()
    movie = scratch / "input.swf"
    movie.write_bytes(input_fws)
    library = str(ROOT / "toolchain/ffdec/lib/*")
    javac = JAVA.with_name("javac.exe")
    subprocess.run([javac, "-encoding", "UTF-8", "-cp", library, "-d", scratch,
                    HERE/"MyProBattlePatch.java", HERE/"MyProNativeAutoPatch.java"], check=True)
    original = scratch / "original.abc"
    patched = scratch / "patched.abc"
    patcher = [JAVA, "-cp", str(scratch) + os.pathsep + library, "MyProNativeAutoPatch"]
    subprocess.run([*patcher, movie, original, patched], check=True)
    original_bytes = original.read_bytes()
    matches = [(offset, payload, raw) for offset, payload, raw in tags
               if payload[payload.index(0, 4) + 1:] == original_bytes]
    if len(matches) != 1:
        raise ValueError("Cannot associate patched native button with its original ABC")
    offset, payload, original_tag = matches[0]
    source = Path(shutil.copyfile(HERE/"MyProNativeAuto.as", scratch/"MyProNativeAuto.as"))
    globals_dir = ENGINE / "core/src/avm2/globals"
    subprocess.run([JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler",
                    "-optimize", "-builtin", "-apiversioning", "-outdir", scratch, "-out", "playerglobal",
                    globals_dir/"Toplevel.as", globals_dir/"globals.as"], check=True)
    subprocess.run([JAVA, "-jar", ASC, "-AS3", "-import", scratch/"playerglobal.abc", source], check=True)
    helper = abc_tag(struct.pack("<I", 1) + TAG_NAME + source.with_suffix(".abc").read_bytes())
    game = abc_tag(payload[:payload.index(0, 4) + 1] + patched.read_bytes())
    result = body[:offset] + helper + game + body[offset + len(original_tag):]
    output = input_fws[:4] + struct.pack("<I", len(result) + 8) + result
    checked = scratch / "checked.swf"
    checked.write_bytes(output)
    subprocess.run([*patcher, movie, checked], check=True)
    return output
