"""Build the fixed start callbacks and validate the locally archived live control-bar skins."""
import argparse
import lzma
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

from observe_skin_motion import ASC, ENGINE, JAVA, ROOT, iter_tags, unpack_swf


HERE = Path(__file__).resolve().parent
ARCHIVE = ROOT / "resources/swf-reference"
START = ARCHIVE / "02-originals/official/start~2107300001.swf"
CONTROL = ARCHIVE / "02-originals/official/controlv2~2026082731219998.swf"
LIVE_CONTROL = ARCHIVE / "01-current/Patches/controlv2-theme-live-2026082731219998.fws.swf"
HELPER_NAME = b"aola.tools.MyProGameUi"
PROFILES = ENGINE / "frontend-utils/src/backends/navigator"


def compile_globals(work):
    globals_dir = ENGINE / "core/src/avm2/globals"
    subprocess.run([JAVA,"-classpath",ASC,"macromedia.asc.embedding.ScriptCompiler","-optimize",
                    "-builtin","-apiversioning","-outdir",work,"-out","playerglobal",
                    globals_dir / "Toplevel.as",globals_dir / "globals.as"], check=True)

def add_script(work, original, helper):
    source = Path(shutil.copyfile(HERE / f"{helper}.as", work / f"{helper}.as"))
    subprocess.run([JAVA,"-jar",ASC,"-AS3","-import",work / "playerglobal.abc",source],check=True)
    body = unpack_swf(original)
    tags = list(iter_tags(body))
    (offset,) = [at for kind,at,_,_ in tags if kind == 82]
    # A non-lazy script registers callbacks once when the first frame initializes.
    payload = bytes(4) + f"aola.tools.{helper}".encode() + b"\0" + source.with_suffix(".abc").read_bytes()
    tag = struct.pack("<HI",(82 << 6) | 63,len(payload)) + payload
    patched = body[:offset] + tag + body[offset:]
    candidate = b"FWS" + original[3:4] + struct.pack("<I",len(patched)+8) + patched
    verify = list(iter_tags(unpack_swf(candidate)))
    remaining = [raw for kind,_,p,raw in verify if not (kind == 82 and p == payload)]
    if len(verify) != len(tags)+1 or remaining != [t[3] for t in tags]:
        raise ValueError("Original tags changed while attaching the UI script")
    return candidate


def profile_rows(packed, candidate):
    """Copy exact spans of the official FWS, embedding only unmatched bytes."""
    body=unpack_swf(packed)
    original=b"FWS"+packed[3:8]+body
    # A fixed-stride block index keeps comparisons linear even for repeated artwork.
    by_block={original[i:i+64]:i for i in range(0,len(original)-63,32)}
    rows=bytearray()
    rebuilt=bytearray()
    cursor=plain=0
    while cursor+64<=len(candidate):
        at=by_block.get(candidate[cursor:cursor+64])
        if at is None:
            cursor+=1
            continue
        left=cursor
        while left>plain and at and candidate[left-1]==original[at-1]:
            left-=1
            at-=1
        if left>plain:
            data=candidate[plain:left]
            rows.extend(struct.pack("<II",0xffffffff,len(data))+data)
            rebuilt.extend(data)
        end=cursor+64
        offset=at+(end-left)
        while end<len(candidate) and offset<len(original) and candidate[end]==original[offset]:
            end+=1
            offset+=1
        rows.extend(struct.pack("<II",at,end-left))
        rebuilt.extend(original[at:offset])
        plain=cursor=end
    if plain<len(candidate):
        data=candidate[plain:]
        rows.extend(struct.pack("<II",0xffffffff,len(data))+data)
        rebuilt.extend(data)
    if rebuilt!=candidate:
        raise ValueError("UI edit profile failed to reconstruct the candidate")
    return rows


def build_bundle(bundle,emit_profile=False):
    bundle = Path(bundle)
    bundle.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory() as tmp:
        work=Path(tmp)
        compile_globals(work)
        start=START.read_bytes()
        candidate=add_script(work,start,"MyProGameUi")
        control_ui=add_script(work,LIVE_CONTROL.read_bytes(),"MyProGameUiDomain")
    (bundle / "start-ui.swf").write_bytes(candidate)
    shutil.copyfile(START,bundle / "start-official.swf")
    official = list(iter_tags(unpack_swf(CONTROL.read_bytes())))
    control = LIVE_CONTROL.read_bytes()
    reviewed = list(iter_tags(unpack_swf(control)))
    if ([k for k,_,_,_ in official] != [k for k,_,_,_ in reviewed]
            or [r for k,_,_,r in official if k!=82] != [r for k,_,_,r in reviewed if k!=82]
            or not all(s in control for s in (b"WaitingBattleEnd",b"applyLocalTheme",b"applyTheme",b"setThemeListeners"))):
        raise ValueError("Unexpected control-bar live theme profile")
    shutil.copyfile(CONTROL,bundle / "control-official.swf")
    (bundle / "control-ui.swf").write_bytes(control_ui)
    for skin in range(1,11):
        path = ARCHIVE / "05-retained/Themes/Live" / f"skin{skin}.swf"
        blob = path.read_bytes()
        expected = f"aola.theme.t{skin}.mmo.controlv2.controlbarskin.skin.Skin{skin}".encode()+b"\0"
        if not any(k == 76 and expected in p for k,_,p,_ in iter_tags(unpack_swf(blob))):
            raise ValueError(f"Missing live theme export for skin{skin}")
        (bundle / path.name).write_bytes(blob)
    for name,packed,new in [("start",start,candidate),("control",CONTROL.read_bytes(),control_ui)]:
        path=PROFILES / f"aola_game_ui_{name}.edits.lzma"
        edits=profile_rows(packed,new)
        if emit_profile:
            path.write_bytes(lzma.compress(edits,format=lzma.FORMAT_ALONE,preset=6))
        elif lzma.decompress(path.read_bytes(),format=lzma.FORMAT_ALONE)!=edits:
            raise ValueError(f"{name} candidate differs from the verified release profile")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir",required=True,type=Path)
    parser.add_argument("--emit-profile",action="store_true")
    args = parser.parse_args()
    build_bundle(args.output_dir,args.emit_profile)
