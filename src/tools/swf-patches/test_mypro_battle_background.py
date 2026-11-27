"""Run the serialized background helper with a local JPEG and a mock desktop bridge."""
import argparse
import base64
import io
import os
from pathlib import Path
import struct
import subprocess

from PIL import Image
from observe_skin_motion import ASC, ENGINE, JAVA, iter_tags, unpack_swf


def run(candidate, runner, work):
    fixture = work / "tests/swfs/mypro_battle_background"
    fixture.mkdir(parents=True, exist_ok=True)
    image = io.BytesIO()
    Image.new("RGB", (960, 560), (230, 24, 48)).save(image, "JPEG", optimize=True)
    source = work / "MyProBackgroundProbe.as"
    template = Path(__file__).with_name("harness") / source.name
    source.write_text(template.read_text().replace("@JPEG@", base64.b64encode(image.getvalue()).decode()), encoding="ascii")
    globals_dir = ENGINE / "core/src/avm2/globals"
    subprocess.run([JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize", "-builtin",
                    "-apiversioning", "-outdir", work, "-out", "playerglobal",
                    globals_dir / "Toplevel.as", globals_dir / "globals.as"], check=True)
    subprocess.run([JAVA, "-jar", ASC, "-AS3", "-import", work / "playerglobal.abc",
                    "-swf", "MyProBackgroundProbe,100,100,30", source], check=True)
    game = unpack_swf(candidate.read_bytes())
    helper, = (payload for kind, _, payload, _ in iter_tags(game)
               if kind == 82 and payload[4:].startswith(b"aola.tools.MyProBattleBackground\0"))
    if helper.count(b"flash.external") != 1:
        raise ValueError("Unexpected host namespace")
    helper = helper.replace(b"flash.external", b"probe.external")
    body = unpack_swf(source.with_suffix(".swf").read_bytes())
    _, start, _, _ = next(t for t in iter_tags(body) if t[0] in (72, 82))
    body = body[:start] + struct.pack("<HI", (82 << 6) | 63, len(helper)) + helper + body[start:]
    (fixture / "test.swf").write_bytes(b"FWS\x12" + struct.pack("<I", len(body) + 8) + body)
    # JPEG loading finishes on the worker executor. Give it wall time while
    # advancing the normal fixture frames, including on a loaded build machine.
    (fixture / "test.toml").write_text("num_frames = 45\nsleep_to_meet_frame_rate = true\n")
    expected = """bind 1 true
pending scene change 1 1
decoded 960 560 true true
skill layer true true
official before update false
custom after update true
detached 2 false true
second battle true 1
restore 1 false 2
invalid 1 false 3
unchanged revision 3
new image true 4
"""
    (fixture / "output.txt").write_text(expected)
    result = subprocess.run([runner, "mypro_battle_background", "--exact"], cwd=work, capture_output=True,
                            timeout=45, env={**os.environ, "RUFFLE_TEST_OPTS": "--compile-mode use-precompiled"})
    output = result.stdout + result.stderr
    (work / "runner.log").write_bytes(output)
    print(output.decode(errors="replace"))
    if result.returncode or b"1 passed" not in output:
        raise SystemExit(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("candidate", "runner", "work"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    run(args.candidate.resolve(strict=True), args.runner.resolve(strict=True), args.work.resolve())
