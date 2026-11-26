"""Run the captured backpack skin animation in Ruffle's headless test runner."""
import argparse
from datetime import datetime, timezone
import json
import lzma
import os
from pathlib import Path
import shutil
import struct
import subprocess
import zlib

from pilot_resource import REFERENCE, RESOURCE, ROOT, VERSION
from skin_recipe import rust_table

ENGINE = ROOT / "src/optimization/engine"
PROBE = Path(__file__).with_name("harness") / "SkinMotionProbe.as"
JAVA = ROOT / "toolchain/jdk-21.0.12.1+1/bin/java.exe"
ASC = ENGINE / "tools/asc/asc.jar"
ORIGINALS = ROOT / "resources/original"
INTERFACES = ORIGINALS / "library_interfaces~2026102936286521.swf"
TWEEN_LIBRARY = ORIGINALS / "library_util~2026090340574444.swf"
COMMON = ORIGINALS / "library_common~2026102936286521.swf"


def unpack_swf(data):
    size = int.from_bytes(data[4:8], "little") - 8
    if data[:3] == b"FWS":
        body = data[8:]
    elif data[:3] == b"CWS":
        body = zlib.decompress(data[8:])
    elif data[:3] == b"ZWS":
        prop = data[12]
        filters = [dict(id=lzma.FILTER_LZMA1, lc=prop % 9, lp=prop // 9 % 5,
                        pb=prop // 45, dict_size=int.from_bytes(data[13:17], "little"))]
        body = lzma.LZMADecompressor(lzma.FORMAT_RAW, filters=filters).decompress(data[17:])
    else:
        raise ValueError("Unsupported SWF signature")
    if len(body) != size:
        raise ValueError("SWF length mismatch")
    return body


def iter_tags(body):
    pos = (5 + 4 * (body[0] >> 3) + 7) // 8 + 4  # RECT, rate, frames
    while pos < len(body):
        start = pos
        kind, size = divmod(struct.unpack_from("<H", body, pos)[0], 64)
        pos += 2
        if size == 63:
            size = struct.unpack_from("<I", body, pos)[0]
            pos += 4
        if pos + size > len(body):
            raise ValueError("Truncated SWF tag")
        yield kind, start, body[pos:pos + size], body[start:pos + size]
        pos += size


def abc_tags(data, include):
    selected = []
    for kind, _, payload, raw in iter_tags(unpack_swf(data)):
        if kind == 82 and include(payload[4:].split(b"\0", 1)[0]):
            if not struct.unpack_from("<I", payload)[0] & 1:
                raise ValueError("Unexpected eager game initializer")
            selected.append(raw)
    if not selected:
        raise ValueError("Missing required ABC")
    return selected


def compile_fixture(work, package):
    globals_dir = ENGINE / "core/src/avm2/globals"
    commands = [
        [JAVA, "-classpath", ASC, "macromedia.asc.embedding.ScriptCompiler",
         "-optimize", "-builtin", "-apiversioning", "-outdir", work, "-out", "playerglobal",
         globals_dir / "Toplevel.as", globals_dir / "globals.as"],
        [JAVA, "-jar", ASC, "-AS3", "-import", work / "playerglobal.abc",
         "-swf", "SkinMotionProbe,160,120,30", work / PROBE.name],
    ]
    shutil.copyfile(PROBE, work / PROBE.name)
    for i, command in enumerate(commands):
        with (work / f"compiler-{i}.log").open("wb") as log:
            subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    artwork = (work / "SkinMotionProbe.swf").read_bytes()
    body = unpack_swf(artwork)
    offset = next(start for kind, start, _, _ in iter_tags(body) if kind in (72, 82))
    # The backpack stores unrelated services and controls in the same DoABC.
    # Link their interfaces and base classes even though the test never opens them.
    interfaces = {b"mmo/interfaces/" + name for name in
                  (b"IService", b"IDispose", b"IShowPanel", b"IClosePanel", b"IInitService",
                   b"pet/petpackage/IPetPackageService")}
    abc = abc_tags(INTERFACES.read_bytes(), interfaces.__contains__)
    abc += abc_tags(TWEEN_LIBRARY.read_bytes(), lambda name: name.startswith(b"com/greensock/"))
    abc += abc_tags(COMMON.read_bytes(), lambda name: name == b"mmo/common/dialog/DialogBase")
    abc += abc_tags(package, lambda name: True)
    body = body[:offset] + b"".join(abc) + body[offset:]
    return b"FWS" + bytes([18]) + struct.pack("<I", len(body) + 8) + body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", type=Path, required=True, help="Ruffle tests regression_tests executable")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--candidate", type=Path, help="Local SWF under test, expected to stop owned tweens")
    source.add_argument("--control", type=Path, help="No-change rebuild of the captured original")
    args = parser.parse_args()
    runner = args.runner.resolve(strict=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    work = ROOT / "build/swf-patches" / f"harness-{stamp}"
    fixture = work / "tests/swfs/skin_motion"
    fixture.mkdir(parents=True)
    target = (args.candidate or args.control or REFERENCE).resolve(strict=True)
    reference = REFERENCE.read_bytes()
    package = reference if target == REFERENCE else target.read_bytes()
    if args.control and unpack_swf(package) != unpack_swf(reference):
        raise ValueError("No-change control differs from the captured movie after decompression")
    if args.candidate:
        recipe = rust_table(unpack_swf(reference), unpack_swf(package))
        approved = ENGINE / "frontend-utils/src/backends/navigator/aola_skin_recipe.rs"
        if recipe not in approved.read_text(encoding="utf-8"):
            raise ValueError("Candidate does not apply the reviewed edits to the captured version")
    (fixture / "test.swf").write_bytes(compile_fixture(work, package))
    (fixture / "test.toml").write_text("num_frames = 345\nsleep_to_meet_frame_rate = true\n", encoding="utf-8")
    expected_active = 0 if args.candidate else 30
    expected_rapid = "0 foreignPreserved=true reopenedSurvived=true" if args.candidate else (
        "3 foreignPreserved=true reopenedSurvived=false")
    expected = ("naturalOut=-1000,0|1000,0|0,-1000|0,1000|10,20\n"
                "naturalBack=0,0|0,0|0,0|0,0|0,0\n"
                f"cycles=30 activeAtClose={expected_active} movedAfterClose={expected_active} foreignActive=30\n"
                "foreignDoneIncludingWarmup=35 retainedKeys=0\n"
                "foreignExactlyOnceAtFinalAlpha=35\n"
                f"rapidOwnedBefore=3 rapidOwnedAfter={expected_rapid}\n"
                f"lateRegistrationTargets={0 if args.candidate else 2}\n"
                f"styleTypesActiveBefore=5 styleTypesActiveAfter={0 if args.candidate else 5}\n"
                f"styleTypesMovedAfterClose={0 if args.candidate else 5}\n")
    (fixture / "output.txt").write_text(expected, encoding="utf-8")
    env = dict(os.environ, RUST_LOG="error", RUFFLE_TEST_OPTS="--compile-mode use-precompiled",
               AOLA_SUBTREE_SCRIPT_SLEEP="0", AOQI_STAGE3_MODE="sleep")
    result = subprocess.run([runner, "skin_motion", "--exact"], cwd=work, env=env,
                            capture_output=True, timeout=90)
    log = (result.stdout + result.stderr).decode("utf-8", errors="replace")
    (work / "runner.log").write_text(log, encoding="utf-8")
    passed = result.returncode == 0 and "1 passed" in log and " ERROR " not in log
    report = {
        "dateUtc": stamp, "runner": str(runner), "result": "pass" if passed else "fail",
        "resource": RESOURCE, "referenceVersion": VERSION, "reference": str(REFERENCE),
        "referenceBytes": len(reference), "package": str(target), "packageBytes": len(package),
        "fixtureLibraries": [p.name for p in (INTERFACES, TWEEN_LIBRARY, COMMON)],
        "group": "B" if args.candidate else ("roundtrip" if args.control else "A"),
        "scope": "Isolated package DoABC with Sprite artwork, headless, no official client or GPU",
        "frameRate": 30, "warmup": 5, "measured": 30, "finalObservationFrame": 336,
        "stage3Mode": env["AOQI_STAGE3_MODE"], "subtreeScriptSleep": False,
        "verifiedTrace": expected.splitlines() if passed else None, "outputs": str(work),
    }
    evidence = ROOT / "doc/evidence/swf-lifecycle" / f"harness-{stamp}"
    evidence.mkdir(parents=True)
    (evidence / "report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding="utf-8")
    print(evidence)
    if not passed:
        raise SystemExit(f"Fixture failed. See {work / 'runner.log'}")


if __name__ == "__main__":
    main()
