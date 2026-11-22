"""Package the Aola desktop build and its versioned notice and battle resources."""
import argparse
import mmap
import shutil
import tempfile
from pathlib import Path

from capture_package import copy_licenses
from pilot_resource import ROOT


NOTICE_VERSION = "2026062579184482"
NOTICE_ORIGINAL = (
    ROOT / "resources/swf-reference/notice" / NOTICE_VERSION
    / f"messagesendservice~{NOTICE_VERSION}.swf"
)
BATTLE_VERSION = "261112849126071"
BATTLE_ORIGINAL = ROOT / "resources/original" / f"petfightnew~{BATTLE_VERSION}.swf"


def write_launcher(path, *, background_zws, hide_notice, block_skill):
    lines = [
        "@echo off",
        "setlocal",
        'set "AOQI_STAGE3_MODE=sleep"',
        'set "AOLA_SWF_LIFECYCLE_MODE=official"',
        'set "AOLA_PET_PATCH="',
        'set "AOLA_SUBTREE_SCRIPT_SLEEP=0"',
        'set "AOLA_GOTO_LEAF_FAST_PATH=1"',
        'set "AOLA_DETAIL_PROFILE=summary"',
        'set "AOLA_GOTO_PROFILE=0"',
        'set "AOLA_ORPHAN_CENSUS=0"',
        'if not defined AOLA_DISK_CACHE_BUDGET_MB set "AOLA_DISK_CACHE_BUDGET_MB=4096"',
        f'set "AOLA_BACKGROUND_ZWS={int(background_zws)}"',
        f'set "AOLA_HIDE_NOTICE={int(hide_notice)}"',
        f'set "AOLA_BLOCK_SKILL={int(block_skill)}"',
        '"%~dp0aola-native-test.exe" --aola-host --frame-rate 50 --aola-data-dir "%~dp0aola-data"',
        "exit /b %errorlevel%",
    ]
    path.write_bytes(("\r\n".join(lines) + "\r\n").encode("ascii"))


def package(output, executable):
    output = output.resolve()
    if output.exists():
        raise FileExistsError(f"Choose a new output directory: {output}")
    for original in (NOTICE_ORIGINAL, BATTLE_ORIGINAL):
        if not original.is_file():
            raise FileNotFoundError(f"Missing original game SWF: {original}")
    with executable.open("rb") as source, mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as image:
        for marker in (b"aolaHideNotice", b"aolaBlockSkill",
                       b"AOLA_DISK_CACHE_BUDGET_MB", b"AOLA_DXGI_CACHE_PRESSURE", b"AOLA_BACKGROUND_ZWS"):
            if image.find(marker) < 0:
                raise ValueError(f"Desktop executable lacks {marker.decode()}: {executable}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as work:
        staging = Path(work) / "package"
        for folder, original in (("swf-notice", NOTICE_ORIGINAL), ("swf-battle", BATTLE_ORIGINAL)):
            assets = staging / folder
            assets.mkdir(parents=True)
            shutil.copyfile(original, assets / "original.swf")
        shutil.copyfile(executable, staging / "aola-native-test.exe")
        copy_licenses(staging)
        for filename, background, hide, block in (
            ("run-game.cmd", True, False, False),
            ("run-control.cmd", False, False, False),
            ("run-hide-notices.cmd", True, True, False),
            ("run-skill-fx-off.cmd", True, False, True),
            ("run-reduced-fx.cmd", True, True, True),
        ):
            write_launcher(staging / filename, background_zws=background,
                           hide_notice=hide, block_skill=block)
        staging.rename(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--executable", type=Path, required=True)
    args = parser.parse_args()
    package(args.output, args.executable)
    print(args.output.resolve())
