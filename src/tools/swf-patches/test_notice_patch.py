"""Exercise the six patched methods in a headless AVM2 movie with a controllable host."""
import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess

from observe_skin_motion import ASC, ENGINE, INTERFACES, JAVA, ROOT, abc_tags, iter_tags, unpack_swf


def notice_libraries():
    names = {b"mmo/interfaces/" + part for part in (
        b"IService", b"IInitService", b"IDispose", b"title/TitleEvent", b"pet/pm/services/IPMService",
        b"messagesend/IMessageSendService", b"messagesend/MessageSendInfo")}
    tags = b"".join(abc_tags(INTERFACES.read_bytes(), names.__contains__))
    scene_types = {b"mmo/play/scene/" + name for name in (b"RoomInfo", b"SceneInfo", b"AbstractScene", b"SceneBase")}
    play = ROOT / "resources/original/play~261031358096721.swf"
    return tags + b"".join(abc_tags(play.read_bytes(), scene_types.__contains__))


def run_probe(args, label, class_name, access_java, tags, expected, access_args=(), frames=2):
    work = args.work.resolve()
    fixture = work / "tests/swfs" / label
    fixture.mkdir(parents=True)
    helpers = Path(__file__).resolve().with_name("harness")
    globals_dir = ENGINE / "core/src/avm2/globals"
    candidate = args.candidate.resolve(strict=True)
    source = class_name + ".as"
    shutil.copyfile(helpers / source, work / source)
    commands = [
        [JAVA, "-cp", str(ROOT / "toolchain/ffdec/lib/*"), helpers / access_java, candidate,
         work / "probe.abc", *access_args],
        [JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler", "-optimize", "-builtin",
         "-apiversioning", "-outdir", work, "-out", "playerglobal", globals_dir / "Toplevel.as", globals_dir / "globals.as"],
        [JAVA, "-jar", ASC, "-AS3", "-import", work / "playerglobal.abc", "-swf", f"{class_name},100,100,30",
         work / source],
    ]
    for i, command in enumerate(commands):
        with (work / f"compile-{i}.log").open("wb") as log:
            subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT, check=True)

    # Include the extracted game class (with test access), linked to its local
    # game libraries. Never launch the login movie or initialize its services.
    payload, = (p for k, _, p, _ in iter_tags(unpack_swf(candidate.read_bytes())) if k == 82)
    patched_payload = payload[:payload.index(0, 4) + 1] + (work / "probe.abc").read_bytes()
    tags += struct.pack("<HI", (82 << 6) | 63, len(patched_payload)) + patched_payload
    body = unpack_swf((work / (class_name + ".swf")).read_bytes())
    first = next(start for kind, start, _, _ in iter_tags(body) if kind in (72, 82))
    body = body[:first] + tags + body[first:]
    (fixture / "test.swf").write_bytes(b"FWS\x12" + struct.pack("<I", len(body) + 8) + body)
    (fixture / "test.toml").write_text(f"num_frames = {frames}\n", encoding="utf-8")
    (fixture / "output.txt").write_text(expected, encoding="utf-8")
    result = subprocess.run([args.runner.resolve(strict=True), label, "--exact"],
                            cwd=work, env={**os.environ, "RUFFLE_TEST_OPTS":"--compile-mode use-precompiled"},
                            capture_output=True, timeout=90)
    output = result.stdout + result.stderr
    (work / "runner.log").write_bytes(output)
    print(output.decode("utf-8", errors="replace"))
    if result.returncode or b"1 passed" not in output:
        raise SystemExit(1)


def probe_args(description):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True, help="Fresh directory")
    return parser.parse_args()


if __name__ == "__main__":
    run_probe(probe_args(__doc__), "aola_notice", "NoticeProbe", "notice_probe.java", notice_libraries(),
              "random init true true\nrandom tick true\nempty 0 random,random,honor,title,broadcast\n"
              "clear 0 true coming\nrestored 4 16 body true title,broadcast,coming\n")
