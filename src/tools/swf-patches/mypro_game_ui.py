"""Build start callbacks and assemble the reviewed control-bar theme upgrade."""
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
START_DOCUMENT = b"mmo.checkversion.Load"
CONTROL_DOCUMENT = b"mmo.controlv2.Control"


def compile_globals(work):
    globals_dir = ENGINE / "core/src/avm2/globals"
    subprocess.run([JAVA,"-classpath",ASC,"macromedia.asc.embedding.ScriptCompiler","-optimize",
                    "-builtin","-apiversioning","-outdir",work,"-out","playerglobal",
                    globals_dir / "Toplevel.as",globals_dir / "globals.as"], check=True)

def compile_script(work, helper):
    source = Path(shutil.copyfile(HERE / f"{helper}.as", work / f"{helper}.as"))
    subprocess.run([JAVA,"-jar",ASC,"-AS3","-import",work / "playerglobal.abc",source],check=True)
    return bytes(4) + f"aola.tools.{helper}".encode() + b"\0" + source.with_suffix(".abc").read_bytes()


def add_script(work, original, helper):
    body = unpack_swf(original)
    tags = list(iter_tags(body))
    (offset,) = [at for kind,at,_,_ in tags if kind == 82]
    # A non-lazy script registers callbacks once when the first frame initializes.
    payload = compile_script(work, helper)
    tag = struct.pack("<HI",(82 << 6) | 63,len(payload)) + payload
    patched = body[:offset] + tag + body[offset:]
    candidate = b"FWS" + original[3:4] + struct.pack("<I",len(patched)+8) + patched
    verify = list(iter_tags(unpack_swf(candidate)))
    remaining = [raw for kind,_,p,raw in verify if not (kind == 82 and p == payload)]
    if len(verify) != len(tags)+1 or remaining != [t[3] for t in tags]:
        raise ValueError("Original tags changed while attaching the UI script")
    return candidate


def ui_source(data, expected_document):
    """Read the startup/control tag stream and its single initial AVM2 program."""
    body = unpack_swf(data)
    tags = list(iter_tags(body))
    document = expected_document.decode("ascii")
    programs = [i for i, (kind, _, _, _) in enumerate(tags) if kind in (72, 82)]
    frames = [i for i, (kind, _, _, _) in enumerate(tags) if kind == 1]
    if (not tags or tags[-1][0] != 0 or tags[-1][2] or any(tag[0] == 0 for tag in tags[:-1])
            or len(programs) != 1 or not frames or programs[0] >= frames[0]
            or tags[programs[0]][0] != 82):
        raise ValueError(f"{document}: expected one DoABC2 before the first frame in a complete SWF")
    index = programs[0]
    payload = tags[index][2]
    separator = payload.find(b"\0", 4)
    if separator < 0 or len(payload) - separator < 6:
        raise ValueError(f"{document}: truncated DoABC2 header or ABC program")

    symbols = []
    for kind, _, payload, _ in tags:
        if kind != 76:
            continue
        count = struct.unpack_from("<H", payload)[0]
        offset = 2
        for _ in range(count):
            character = struct.unpack_from("<H", payload, offset)[0]
            end = payload.index(0, offset + 2)
            symbols.append((character, payload[offset + 2:end]))
            offset = end + 1
        if offset != len(payload):
            raise ValueError(f"{document}: unexpected trailing SymbolClass data")
    if [name for character, name in symbols if character == 0] != [expected_document]:
        raise ValueError(f"{document}: document class is missing from SymbolClass")
    return body, tags, index, separator + 1, symbols


def reviewed_control():
    official = CONTROL.read_bytes()
    reviewed = LIVE_CONTROL.read_bytes()
    before = list(iter_tags(unpack_swf(official)))
    after = list(iter_tags(unpack_swf(reviewed)))
    if ([k for k, _, _, _ in before] != [k for k, _, _, _ in after]
            or [r for k, _, _, r in before if k != 82] != [r for k, _, _, r in after if k != 82]
            or not all(s in reviewed for s in
                       (b"WaitingBattleEnd", b"applyLocalTheme", b"applyTheme", b"setThemeListeners"))):
        raise ValueError("Unexpected control-bar live theme profile")
    return official, reviewed


def build_weekly_ui(resource: str, source: Path, work: Path) -> Path:
    work = Path(work).resolve()
    source = Path(source).resolve(strict=True)
    data = source.read_bytes()
    if resource == "start":
        ui_source(data, START_DOCUMENT)
        work.mkdir(parents=True, exist_ok=False)
        helper = "MyProGameUi"
    elif resource == "control":
        body, tags, index, header, symbols = ui_source(data, CONTROL_DOCUMENT)
        official, reviewed = reviewed_control()
        _, original_tags, original_index, original_header, original_symbols = ui_source(official, CONTROL_DOCUMENT)
        _, reviewed_tags, reviewed_index, reviewed_header, _ = ui_source(reviewed, CONTROL_DOCUMENT)
        if sorted(name for _, name in symbols) != sorted(name for _, name in original_symbols):
            raise ValueError("mmo.controlv2.Control SymbolClass exports differ from the reviewed control module")

        work.mkdir(parents=True, exist_ok=False)
        if tags[index][2][header:] != original_tags[original_index][2][original_header:]:
            from weekly_control import build_control_theme

            patched = build_control_theme(source, work / "theme-rebase")
            _, reviewed_tags, reviewed_index, reviewed_header, patched_symbols = ui_source(
                patched.read_bytes(), CONTROL_DOCUMENT)
            if patched_symbols != symbols:
                raise ValueError("mmo.controlv2.Control SymbolClass changed during theme recompilation")
        _, offset, payload, raw = tags[index]
        abc = payload[:header] + reviewed_tags[reviewed_index][2][reviewed_header:]
        program = struct.pack("<HI", (82 << 6) | 63, len(abc)) + abc
        body = body[:offset] + program + body[offset + len(raw):]
        data = b"FWS" + data[3:4] + struct.pack("<I", len(body) + 8) + body
        helper = "MyProGameUiDomain"
    else:
        raise ValueError(f"Unsupported weekly UI resource: {resource}")

    compile_globals(work)
    candidate = work / f"{resource}.fws.swf"
    candidate.write_bytes(add_script(work, data, helper))
    return candidate


def qualify_weekly_ui(resource, candidate, runner, work):
    # Import lazily so the regression harness can import the compiler helpers.
    from test_mypro_game_ui import test_weekly

    test_weekly(resource, candidate, runner, work)


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
        official_control, control = reviewed_control()
        control_ui=add_script(work,control,"MyProGameUiDomain")
    (bundle / "start-ui.swf").write_bytes(candidate)
    shutil.copyfile(START,bundle / "start-official.swf")
    shutil.copyfile(CONTROL,bundle / "control-official.swf")
    (bundle / "control-ui.swf").write_bytes(control_ui)
    for skin in range(1,11):
        path = ARCHIVE / "05-retained/Themes/Live" / f"skin{skin}.swf"
        blob = path.read_bytes()
        expected = f"aola.theme.t{skin}.mmo.controlv2.controlbarskin.skin.Skin{skin}".encode()+b"\0"
        if not any(k == 76 and expected in p for k,_,p,_ in iter_tags(unpack_swf(blob))):
            raise ValueError(f"Missing live theme export for skin{skin}")
        (bundle / path.name).write_bytes(blob)
    for name,packed,new in [("start",start,candidate),("control",official_control,control_ui)]:
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
    parser.add_argument("--weekly-resource", choices=("start", "control"))
    parser.add_argument("--source", type=Path)
    args = parser.parse_args()
    if args.weekly_resource:
        if args.source is None or args.emit_profile:
            parser.error("--weekly-resource requires --source and cannot be used with --emit-profile")
        print(build_weekly_ui(args.weekly_resource, args.source, args.output_dir))
    else:
        if args.source is not None:
            parser.error("--source requires --weekly-resource")
        build_bundle(args.output_dir,args.emit_profile)
