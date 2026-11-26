"""Run serialized battle presentation branches with an offline AVM2 host and game doubles."""
import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess

from mypro_battle_effects import HELPER_NAME, HERE, _doabc
from observe_skin_motion import ASC, ENGINE, JAVA, ROOT, iter_tags, unpack_swf


def run(candidate, runner, work):
    fixture = work / "tests/swfs/mypro_battle_effects"
    fixture.mkdir(parents=True, exist_ok=True)
    source = Path(shutil.copyfile(HERE / "harness/MyProEffectsProbe.as", work / "MyProEffectsProbe.as"))
    globals_dir = ENGINE / "core/src/avm2/globals"
    movie = source.with_suffix(".swf")
    commands = [
        [JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize", "-builtin",
         "-apiversioning", "-outdir", work, "-out", "playerglobal", globals_dir / "Toplevel.as", globals_dir / "globals.as"],
        [JAVA, "-jar", ASC, "-AS3", "-import", work / "playerglobal.abc", "-swf", "MyProEffectsProbe,100,100,30", source],
        [JAVA, "-cp", str(ROOT / "toolchain/ffdec/lib/*"), HERE / "harness/MyProEffectsProbeSites.java",
         candidate, movie, work / "sites.abc"],
    ]
    for command in commands:
        subprocess.run(command, check=True, cwd=work)
    helper, = [p for kind, _, p, _ in iter_tags(unpack_swf(candidate.read_bytes()))
               if kind == 82 and p[4:].startswith(HELPER_NAME)]
    if helper.count(b"flash.external") != 1:
        raise ValueError("Unexpected host namespace")
    helper = helper.replace(b"flash.external", b"probe.external")
    body = unpack_swf(movie.read_bytes())
    kind, start, payload, raw = next(t for t in iter_tags(body) if t[0] in (72, 82))
    prefix = payload[:payload.index(0, 4) + 1] if kind == 82 else b"\0\0\0\0probe\0"
    body = body[:start] + _doabc(helper) + _doabc(prefix + (work / "sites.abc").read_bytes()) + body[start + len(raw):]
    (fixture / "test.swf").write_bytes(b"FWS\x12" + struct.pack("<I", len(body) + 8) + body)
    (fixture / "test.toml").write_text("num_frames = 2\n", encoding="ascii")
    normal = "switchMovie,skillMovie,turnMovie delay=16"
    skipped = "switchEnd11,refresh,bs1,bg2,up3,skillEnd,turnEnd delay=1"
    expected = [
        "default d,h,m-2,m0,m2," + normal, "damage h,m0,m2," + normal, "healing d,m-2,m0," + normal,
        "correct-rate", "skip d,h,m-2,m0,m2," + skipped, "correct-rate", "all m0," + skipped,
        "host-absent d,h,m-2,m0,m2," + normal, "host-error d,h,m-2,m0,m2," + normal,
    ]
    (fixture / "output.txt").write_text("\n".join(expected) + "\n", encoding="ascii")
    result = subprocess.run([runner, "mypro_battle_effects", "--exact"], cwd=work, capture_output=True,
                            timeout=60, env={**os.environ, "RUFFLE_TEST_OPTS": "--compile-mode use-precompiled"})
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
