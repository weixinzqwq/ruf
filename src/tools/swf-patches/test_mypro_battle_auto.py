"""Run the packaged native auto control in AVM2 against a local button and host."""
import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess

from mypro_battle_auto import HERE, TAG_NAME, abc_tag
from observe_skin_motion import ASC, ENGINE, JAVA, iter_tags, unpack_swf


def run(candidate, runner, work):
    fixture = work / "tests/swfs/mypro_native_auto"
    fixture.mkdir(parents=True, exist_ok=True)
    source = Path(shutil.copyfile(HERE/"harness/MyProNativeAutoProbe.as", work/"MyProNativeAutoProbe.as"))
    globals_dir = ENGINE/"core/src/avm2/globals"
    subprocess.run([JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize",
                    "-builtin", "-apiversioning", "-outdir", work, "-out", "playerglobal",
                    globals_dir/"Toplevel.as", globals_dir/"globals.as"], check=True)
    subprocess.run([JAVA, "-jar", ASC, "-AS3", "-import", work/"playerglobal.abc",
                    "-swf", "MyProNativeAutoProbe,100,100,30", source], check=True)
    game = unpack_swf(candidate.read_bytes())
    helper, = (p for k, _, p, _ in iter_tags(game) if k == 82 and p[4:].startswith(TAG_NAME))
    if helper.count(b"flash.external") != 1:
        raise ValueError("Unexpected ExternalInterface namespace")
    helper = helper.replace(b"flash.external", b"probe.external")
    body = unpack_swf(source.with_suffix(".swf").read_bytes())
    kind, start, payload, raw = next(t for t in iter_tags(body) if t[0] in (72, 82))
    probe = raw if kind == 82 else abc_tag(b"\0\0\0\0probe\0" + payload)
    body = body[:start] + abc_tag(helper) + probe + body[start + len(raw):]
    (fixture/"test.swf").write_bytes(b"FWS\x12" + struct.pack("<I", len(body) + 8) + body)
    (fixture/"test.toml").write_text("num_frames = 3\n", encoding="ascii")
    (fixture/"output.txt").write_text("""no host true
initial ready false false 0
enable applied true true 1
same applied true false 1
disable applied false true 2
wrong type blocked null false 2
cannot auto blocked false false 2
locked blocked false false 2
forced blocked true false 2
mode2 blocked true false 2
mode3 blocked false false 2
won unavailable null false 2
replay unavailable null false 2
no fight unavailable null false 2
disabled button blocked false false 2
hidden panel blocked false false 2
disabled panel blocked false false 2
detached blocked false false 2
disposed true true 2
""", encoding="ascii")
    result = subprocess.run([runner, "mypro_native_auto", "--exact"], cwd=work,
                            env={**os.environ, "RUFFLE_TEST_OPTS":"--compile-mode use-precompiled"},
                            capture_output=True, timeout=45)
    output = result.stdout + result.stderr
    (work/"runner.log").write_bytes(output)
    print(output.decode(errors="replace"))
    if result.returncode or b"1 passed" not in output:
        raise SystemExit(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("candidate", "runner", "work"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    run(args.candidate.resolve(strict=True), args.runner.resolve(strict=True), args.work.resolve())
