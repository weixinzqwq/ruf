"""Build the verified backpack responses for a desktop test package or navigator fixtures."""
import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from pilot_resource import BUILD_ID, REFERENCE, ROOT, UTIL_VERSION
from roundtrip import swf_body


def launcher(mode):
    operations = "1" if mode in ("original", "roundtrip", "lifecycle") else ""
    lines = [
        "@echo off",
        "setlocal",
        f'set "AOLA_SWF_LIFECYCLE_MODE={mode}"',
        f'set "AOLA_SWF_OPERATIONS={operations}"',
        'set "AOLA_PET_PATCH="',
        'set "AOLA_PANEL_DIAGNOSTICS="',
        'set "AOLA_GOTO_PROFILE=1"',
        'set "AOLA_SUBTREE_SCRIPT_SLEEP=0"',
        'set "AOQI_STAGE3_MODE=sleep"',
        'set "AOQI_POOL_IDLE_BUDGET_MB=256"',
        'set "AOQI_POOL_RECENT_FRAMES=120"',
        ':newrun',
        f'set "run=%~dp0runs\\{mode}-%RANDOM%-%RANDOM%"',
        'if exist "%run%\\" goto newrun',
        'mkdir "%run%" || exit /b 1',
        'echo Game data and diagnostics: "%run%"',
    ]
    if mode == "capture":
        lines.append('echo Open the backpack, wait for the panel to load, and exit normally.')
    elif operations:
        lines.append('echo Open the backpack, switch its view, close it promptly, reopen it, then exit normally.')
    lines.extend([
        '"%~dp0aola-native-test.exe" --aola-host --frame-rate 50 --aola-data-dir "%run%"',
        'set "game_status=%errorlevel%"',
    ])
    if mode != "official":
        lines.extend([
            f'python "%~dp0capture_result.py" "%run%" {mode} --game-exit "%game_status%"',
            'if not "%errorlevel%"=="0" exit /b 2',
        ])
    if mode == "capture":
        lines.extend([
            'echo Official backpack and utility SWF responses:',
            'dir /b "%run%\\swf-capture\\*.swf"',
            'echo Saved in "%run%\\swf-capture"',
        ])
    return "\r\n".join([*lines, "exit /b %game_status%", ""]).encode("ascii")


def read_controls(roundtrip):
    original = roundtrip / "reference.swf"
    control = roundtrip / "run-1" / "control.swf"
    original_bytes = original.read_bytes()
    control_bytes = control.read_bytes()
    if original_bytes != REFERENCE.read_bytes():
        raise ValueError("Reference differs from the captured official original")
    if control_bytes != (roundtrip / "run-2" / "control.swf").read_bytes():
        raise ValueError("Independent roundtrip builds differ")
    if swf_body(original_bytes) != swf_body(control_bytes):
        raise ValueError("The roundtrip control changes the movie body")
    return original_bytes, control_bytes


def write_resources(directory, work, roundtrip):
    original, control = read_controls(roundtrip)
    # Rebuild from the captured response and publish only the verifier's final output.
    patch = Path(__file__).with_name("patch_skin.py")
    lifecycle_build = work / "functional-build"
    subprocess.run([sys.executable, patch, "--output-dir", lifecycle_build], check=True)
    directory.mkdir(parents=True)
    (directory / "newpetpackage-original.swf").write_bytes(original)
    (directory / "newpetpackage-roundtrip.swf").write_bytes(control)
    shutil.copyfile(lifecycle_build / "candidate.swf", directory / "newpetpackage-lifecycle.swf")
    util_reference = ROOT / "resources/original" / f"library_util~{UTIL_VERSION}.swf"
    shutil.copyfile(util_reference, directory / util_reference.name)


def copy_licenses(staging):
    engine = ROOT / "src/optimization/engine"
    licenses = staging / "licenses"
    shutil.copytree(engine / "aoqi-assets/resources/licenses", licenses)
    shutil.copyfile(engine / "render/wgpu/LICENSE-AMD-FSR.txt", licenses / "LICENSE-AMD-FSR.txt")
    for license_name in ("MIT", "CC0"):
        shutil.copyfile(engine / f"core/aoqi-gc-arena/LICENSE-{license_name}",
                        licenses / f"gc-arena-LICENSE-{license_name}.txt")
    shutil.copyfile(engine / "LICENSE.md", staging / "RUFFLE-LICENSE.md")


def package(output, roundtrip, executable, fixtures_only=False):
    output = output.resolve()
    if output.exists():
        raise ValueError(f"Choose a new package directory: {output}")
    if not fixtures_only:
        binary = executable.read_bytes()
        if BUILD_ID.encode("ascii") not in binary or b"AOLA_SWF_OPERATIONS" not in binary:
            raise ValueError(f"Expected the optimization desktop build {BUILD_ID} with SWF operation counters")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as work:
        staging = Path(work) / "package"
        write_resources(staging if fixtures_only else staging / "swf-lifecycle", Path(work), roundtrip)
        if fixtures_only:
            staging.rename(output)
            return
        (staging / "aola-native-test.exe").write_bytes(binary)
        shutil.copyfile(Path(__file__).with_name("capture_result.py"), staging / "capture_result.py")
        shutil.copyfile(Path(__file__).with_name("pilot_resource.py"), staging / "pilot_resource.py")
        copy_licenses(staging)
        for mode in ("official", "capture", "original", "roundtrip", "lifecycle"):
            (staging / f"run-{mode}.cmd").write_bytes(launcher(mode))
        staging.rename(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--roundtrip", type=Path, required=True, help="Successful roundtrip.py output")
    parser.add_argument("--output", type=Path, required=True, help="New dist directory")
    parser.add_argument("--fixtures-only", action="store_true", help="Write only the four verified SWF inputs")
    parser.add_argument("--executable", type=Path,
                        default=ROOT / "build/optimization-target/release/ruffle_desktop.exe")
    args = parser.parse_args()
    package(args.output, args.roundtrip, args.executable, args.fixtures_only)
    print(args.output.resolve())


if __name__ == "__main__":
    main()
