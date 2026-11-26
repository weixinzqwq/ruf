"""Apply the optional local presentation guards to an already optimized battle FWS."""
from pathlib import Path
import shutil
import struct
import subprocess

from observe_skin_motion import ASC, ENGINE, JAVA, ROOT, iter_tags

HERE = Path(__file__).resolve().parent
HELPER_NAME = b"aola.tools.MyProBattleEffects\0"


def _doabc(payload):
    return struct.pack("<HI", (82 << 6) | 63, len(payload)) + payload


def apply_effects(input_fws: bytes, work: Path) -> bytes:
    if input_fws[:3] != b"FWS" or int.from_bytes(input_fws[4:8], "little") != len(input_fws):
        raise ValueError("Expected a complete uncompressed battle movie")
    work = (work / "battle-effects").resolve()
    work.mkdir(parents=True, exist_ok=True)
    source = Path(shutil.copyfile(HERE / "MyProBattleEffects.as", work / "MyProBattleEffects.as"))
    original = work / "input.swf"
    original.write_bytes(input_fws)
    globals_dir = ENGINE / "core/src/avm2/globals"
    globals_abc = work.parent / "playerglobal.abc"
    commands = []
    if not globals_abc.is_file():
        globals_abc = work / "playerglobal.abc"
        commands.append([JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize",
                         "-builtin", "-apiversioning", "-outdir", work, "-out", "playerglobal",
                         globals_dir / "Toplevel.as", globals_dir / "globals.as"])
    commands += [
        [JAVA, "-jar", ASC, "-AS3", "-import", globals_abc, source],
        [JAVA, "-cp", str(ROOT / "toolchain/ffdec/lib/*"), HERE / "MyProBattleEffectsPatch.java",
         original, work / "original.abc", work / "patched.abc"],
    ]
    with (work / "build.log").open("wb") as log:
        for command in commands:
            subprocess.run(command, check=True, stdout=log, stderr=subprocess.STDOUT)
    tags = [(offset, payload, raw) for kind, offset, payload, raw in iter_tags(input_fws[8:]) if kind == 82]
    pristine = (work / "original.abc").read_bytes()
    if any(payload[4:].startswith(HELPER_NAME) for _, payload, _ in tags):
        raise ValueError("Battle presentation guards already installed")
    (offset, payload, raw), = [(i, p, r) for i, p, r in tags if p[p.index(0, 4) + 1:] == pristine]
    helper = _doabc(struct.pack("<I", 1) + HELPER_NAME + source.with_suffix(".abc").read_bytes())
    game = _doabc(payload[:payload.index(0, 4) + 1] + (work / "patched.abc").read_bytes())
    body = input_fws[8:8 + offset] + helper + game + input_fws[8 + offset + len(raw):]
    return input_fws[:4] + struct.pack("<I", len(body) + 8) + body
