"""Run the serialized production battle selectors and helper with local AVM2 fixtures."""
import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess
from observe_skin_motion import ASC, ENGINE, JAVA, ROOT, iter_tags, unpack_swf

HERE = Path(__file__).resolve().parent


def tag(payload):
    return struct.pack("<HI", (82 << 6) | 63, len(payload)) + payload


def run(candidate, runner, work):
    fixture = work / "tests/swfs/mypro_battle_model"
    fixture.mkdir(parents=True)
    source = Path(shutil.copyfile(HERE/"harness/MyProModelProbe.as",work/"MyProModelProbe.as"))
    globals_dir = ENGINE/"core/src/avm2/globals"
    movie = source.with_suffix(".swf")
    commands = [
        [JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize", "-builtin",
         "-apiversioning", "-outdir", work, "-out", "playerglobal", globals_dir/"Toplevel.as", globals_dir/"globals.as"],
        [JAVA, "-jar", ASC, "-AS3", "-import", work/"playerglobal.abc", "-swf", "MyProModelProbe,100,100,30", source],
        [JAVA, "-cp", str(ROOT/"toolchain/ffdec/lib/*"), HERE/"harness/MyProModelProbeSites.java", candidate, movie, work/"sites.abc"],
    ]
    for command in commands:
        subprocess.run(command, check=True, cwd=work)
    game = unpack_swf(candidate.read_bytes())
    helper, = (p for kind, _, p, _ in iter_tags(game) if kind == 82 and p[4:].startswith(b"aola.tools.MyProBattleModel\0"))
    # The sole change to the real helper is the host class namespace in its
    # constant pool. AVM2 instructions, resource checks and callbacks are untouched.
    if helper.count(b"flash.external") != 1:
        raise ValueError("Unexpected ExternalInterface namespace")
    helper = helper.replace(b"flash.external", b"probe.external")
    body = unpack_swf(movie.read_bytes())
    kind, start, payload, raw = next(t for t in iter_tags(body) if t[0] in (72,82))
    prefix = payload[:payload.index(0,4)+1] if kind == 82 else b"\0\0\0\0probe\0"
    probe = prefix + (work/"sites.abc").read_bytes()
    body = body[:start] + tag(helper) + tag(probe) + body[start+len(raw):]
    (fixture/"test.swf").write_bytes(b"FWS\x12" + struct.pack("<I",len(body)+8) + body)
    (fixture/"test.toml").write_text("num_frames = 26\n", encoding="ascii")
    (fixture/"output.txt").write_text(
        "body 29 body,17,29,true,true 17\nmissing side 17 body,17,29,false,false 17\n"
        "icon 29 icon,17,29,false,true 1\nmissing target 17 icon,17,87,false,false\n"
        "stale rule 17 body,17,29,true,true\nno host 17 17 3\n"
        "bitmap 31 31 1\npartial bitmap 17\ncold 17 17 0\nprepared 47 mock/47\n"
        "queued stale 17 17\nstale requests 1\nsplit cold 17 config-missing\n"
        "split ready 6044 fightassets/bitmap/pet6044/pet6044_2_0,fightassets/bitmap/pet6044/pet6044_2_1,fightassets/bitmap/pet6044/pet6044_2_2 loading/1/3,loading/2/3,loading/3/3,loaded/3/3\n"
        "partially warm 17 frame-missing\n"
        "completed partial 6044 fightassets/bitmap/pet6044/pet6044_1_0,fightassets/bitmap/pet6044/pet6044_1_1,fightassets/bitmap/pet6044/pet6044_1_2\n", encoding="ascii")
    result = subprocess.run([runner,"mypro_battle_model","--exact"],cwd=work,
                            env={**os.environ,"RUFFLE_TEST_OPTS":"--compile-mode use-precompiled"},
                            capture_output=True,timeout=45)
    output = result.stdout + result.stderr
    (work/"runner.log").write_bytes(output)
    print(output.decode(errors="replace"))
    if result.returncode or b"1 passed" not in output:
        raise SystemExit(1)


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("candidate","runner","work"):
        parser.add_argument("--"+name,type=Path,required=True)
    args=parser.parse_args()
    run(args.candidate.resolve(strict=True),args.runner.resolve(strict=True),args.work.resolve())
