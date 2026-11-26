"""Attach the current, owner-bound lineup to the open official backpack."""
import shutil
import struct
import subprocess
from pathlib import Path

from mypro_battle_auto import abc_tag
from observe_skin_motion import ASC, ENGINE, JAVA, ROOT, iter_tags

HERE = Path(__file__).resolve().parent
TAG_NAME = b"aola.tools.MyProLineup\0"


def apply_lineup(input_fws: bytes, work: Path) -> bytes:
    work = Path(work) / "lineup"
    work.mkdir(parents=True, exist_ok=True)
    if input_fws[:3] != b"FWS" or int.from_bytes(input_fws[4:8], "little") != len(input_fws):
        raise ValueError("Expected complete uncompressed backpack")
    source, old, new = (work / name for name in ("source.swf", "before.abc", "after.abc"))
    source.write_bytes(input_fws)
    patch = [JAVA, "-cp", str(ROOT/"toolchain/ffdec/lib/*"), HERE/"MyProLineupPatch.java"]
    subprocess.run([*patch, source, old, new], check=True)
    abc = old.read_bytes()
    (offset, payload, tag), = [(pos, p, t) for k,pos,p,t in iter_tags(input_fws[8:])
                               if k==82 and p[p.index(0,4)+1:] == abc]
    code = work/"MyProLineup.as"
    shutil.copyfile(HERE/code.name, code)
    globals_dir = ENGINE/"core/src/avm2/globals"
    subprocess.run([JAVA,"-classpath",ASC,"macromedia.asc.embedding.ScriptCompiler","-optimize","-builtin",
                    "-apiversioning","-outdir",work,"-out","playerglobal",globals_dir/"Toplevel.as",globals_dir/"globals.as"],check=True)
    subprocess.run([JAVA,"-jar",ASC,"-AS3","-import",work/"playerglobal.abc",code],check=True)
    helpers = abc_tag(struct.pack("<I", 1) + TAG_NAME + code.with_suffix(".abc").read_bytes())
    game = abc_tag(payload[:payload.index(0,4)+1] + new.read_bytes())
    body = input_fws[8:8+offset] + helpers + game + input_fws[8+offset+len(tag):]
    candidate = input_fws[:4] + struct.pack("<I",8+len(body)) + body
    output = work/"result.swf"
    output.write_bytes(candidate)
    subprocess.run([*patch,source,output],check=True)
    return candidate
