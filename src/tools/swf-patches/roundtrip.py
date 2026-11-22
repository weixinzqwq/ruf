"""Build two unmodified PackageSkinManager.dispose method controls from the archived original."""

import argparse
from datetime import datetime, timezone
import json
import lzma
import os
from pathlib import Path
import re
import struct
import subprocess
import textwrap
from pilot_resource import REFERENCE, RESOURCE, ROOT, VERSION
WORK = ROOT / "build/swf-patches/roundtrip-pet-skin"
CLASS = "mmo.petpackage.controller.skin.PackageSkinManager"
BODY_ID = 220
METHOD_ID = 224
JAVA = ROOT / "toolchain/jdk-21.0.12.1+1/bin/java.exe"
FFDEC = ROOT / "toolchain/ffdec/ffdec-cli.jar"
FFDEC_VERSION = "JPEXS Free Flash Decompiler v.24.1.0 nightly build 3326"
JAVA_VERSION = "21.0.12.1+1-LTS"


def swf_body(data):
    """Decode this pilot's ZWS container. Check the declared sizes before comparing bytes."""
    if len(data) < 18 or data[:4] != b"ZWS\x12":
        raise ValueError("Expected ZWS, SWF version 18")
    declared_size, compressed_size = struct.unpack_from("<II", data, 4)
    if compressed_size != len(data) - 17:
        raise ValueError("Invalid ZWS compressed size")
    properties = data[12]
    filters = [{
        "id": lzma.FILTER_LZMA1,
        "lc": properties % 9,
        "lp": (properties // 9) % 5,
        "pb": properties // 45,
        "dict_size": int.from_bytes(data[13:17], "little"),
    }]
    decoder = lzma.LZMADecompressor(format=lzma.FORMAT_RAW, filters=filters)
    body = decoder.decompress(data[17:])
    if not decoder.eof or decoder.unused_data or len(body) + 8 != declared_size:
        raise ValueError("Incomplete ZWS stream or inconsistent FileLength")
    return body


def tag_counts(body):
    rect_bytes = (5 + 4 * (body[0] >> 3) + 7) // 8
    pos = rect_bytes + 4  # RECT, frame rate, frame count
    count = abc = 0
    while pos + 2 <= len(body):
        header = int.from_bytes(body[pos:pos + 2], "little")
        kind, length = divmod(header, 64)
        pos += 2
        if length == 63:
            length, = struct.unpack_from("<I", body, pos)
            pos += 4
        pos += length
        count += 1
        abc += kind in (72, 82)
        if kind == 0 and pos == len(body) and length == 0:
            return count, abc
    raise ValueError("Missing or malformed final End tag")


def jpexs(run, step, *args):
    # Keep FFDec's config/logs isolated from the user's installed copy and the other build.
    result = subprocess.run(
        [JAVA, "-Djava.awt.headless=true", "-jar", FFDEC, *args], cwd=run,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        env={**os.environ, "APPDATA": str(run)}, check=False,
    )
    log = run / f"{step}.log"
    log.write_bytes(result.stdout)
    if result.returncode:
        raise ValueError(f"FFDec failed during {step} (see {log})")
    return result.stdout.decode("utf-8", errors="replace")


def rebuild(run, source):
    run.mkdir()
    selector = ["-onerror", "abort", "-selectclass", CLASS]
    info = jpexs(run, "coordinates", "-config", "showMethodBodyId=true", *selector,
                 "-export", "script", "as", source)
    if FFDEC_VERSION not in info:
        raise ValueError(f"Expected {FFDEC_VERSION}")
    class_path = Path("scripts", *CLASS.split("."))
    source_text = (run / "as" / class_path.with_suffix(".as")).read_text("utf-8-sig")
    coordinate = rf"public function dispose\(\) : void\s*\{{\s*// method body index: {BODY_ID} method index: {METHOD_ID}\b"
    if len(re.findall(coordinate, source_text)) != 1:
        raise ValueError("Could not identify the locked method and body in the original")

    jpexs(run, "disassemble", *selector, "-format", "script:pcode", "-export", "script", "pcode", source)
    assembly = (run / "pcode" / class_path.with_suffix(".pcode")).read_text("utf-8-sig")
    trait = (r'^[ \t]*public function dispose\(\) : void\s*\{\s*'
             r'trait method QName\(PackageNamespace\(""\),"dispose"\)\s*dispid 0\s*\n'
             r'(?P<indent>[ \t]+)(?P<method>method\s*\n[\s\S]*?^\s*end ; method)[ \t]*$')
    matches = list(re.finditer(trait, assembly, flags=re.MULTILINE))
    if len(matches) != 1:
        raise ValueError("Expected exactly one public dispose():void P-code method")
    method = textwrap.dedent(matches[0]["indent"] + matches[0]["method"]).strip() + "\n"
    (run / "dispose.pcode").write_bytes(method.encode("utf-8"))
    log = jpexs(run, "assemble", "-onerror", "abort", "-replace", source, "control.swf",
                CLASS, "dispose.pcode", str(BODY_ID))
    if "Replace AS3 PCode" not in log:
        raise ValueError("FFDec did not report an AS3 P-code replacement")
    return (run / "control.swf").read_bytes()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, help="Fresh directory for this run (must not exist)")
    args = parser.parse_args()
    default_dir = WORK / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    output = (args.output_dir or default_dir).resolve()
    java_info = subprocess.run([JAVA, "-version"], capture_output=True, check=True).stderr
    if b"Temurin-21.0.12.1+1 " not in java_info or f"(build {JAVA_VERSION})".encode() not in java_info:
        raise ValueError(f"Expected Temurin {JAVA_VERSION}")
    reference = REFERENCE.read_bytes()
    original_body = swf_body(reference)
    tags, abc = tag_counts(original_body)
    output.mkdir(parents=True, exist_ok=False)
    source = output / "reference.swf"
    source.write_bytes(reference)
    first = rebuild(output / "run-1", source)
    second = rebuild(output / "run-2", source)
    if source.read_bytes() != reference:
        raise ValueError("Reference copy changed during rebuild")
    if first != second:
        raise ValueError("Independent rebuilds differ")
    if swf_body(first) != original_body:
        raise ValueError("The rebuilt movie differs from the original after decompression")
    report = {
        "role": "unmodified method control for the archived version",
        "source": REFERENCE.relative_to(ROOT).as_posix(),
        "resource": RESOURCE, "version": VERSION,
        "class": CLASS, "method": "public dispose():void",
        "method_index": METHOD_ID, "method_body_index": BODY_ID,
        "tools": {"ffdec": FFDEC_VERSION, "java": "Temurin " + JAVA_VERSION},
        "source_bytes": len(reference), "control_bytes": len(first),
        "decompressed_body_bytes": len(original_body), "top_level_tags": tags, "abc_tags": abc,
        "two_rebuilds_byte_identical": True,
        "source_and_control_compressed_bytes_identical": first == reference,
        "source_and_control_movie_header_and_all_tag_bytes_identical": True,
        "runtime_tested": False,
    }
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(output / "report.json")


if __name__ == "__main__":
    main()
