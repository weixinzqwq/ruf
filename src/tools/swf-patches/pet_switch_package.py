"""Build the standalone pet switching A/B desktop distribution."""
import argparse
from pathlib import Path
import shutil
import tempfile

from capture_package import copy_licenses
from pet_switch_result import MODES, PERF_BUILD
from pilot_resource import BUILD_ID, REFERENCE, ROOT, UTIL_VERSION


def batch(lines):
    return ("\r\n".join(lines) + "\r\n").encode("utf8")


def launcher(mode):
    fast_path, detail = MODES[mode]
    deep = detail == "full"
    return batch([
        "@echo off",
        "chcp 65001 >nul",
        "setlocal",
        'set "AOLA_SWF_LIFECYCLE_MODE=original"',
        'set "AOLA_SWF_OPERATIONS="',
        'set "AOLA_PET_PATCH="',
        'set "AOLA_PANEL_DIAGNOSTICS="',
        f'set "AOLA_GOTO_LEAF_FAST_PATH={int(fast_path)}"',
        f'set "AOLA_DETAIL_PROFILE={detail}"',
        f'set "AOLA_GOTO_PROFILE={int(deep)}"',
        f'set "AOLA_ORPHAN_CENSUS={int(deep)}"',
        'set "AOLA_SUBTREE_SCRIPT_SLEEP=0"',
        'set "AOQI_STAGE3_MODE=sleep"',
        'set "AOQI_POOL_IDLE_BUDGET_MB=256"',
        'set "AOQI_POOL_RECENT_FRAMES=120"',
        'set "AOQI_GC_SLEEP_FACTOR="',
        'set "AOQI_GC_SLICE_MS="',
        ":newrun",
        f'set "run=%~dp0runs\\{mode}-%RANDOM%-%RANDOM%"',
        'if exist "%run%\\" goto newrun',
        'mkdir "%run%" || exit /b 1',
        f'echo 亚比切换实验 {mode} 组（{"B 组深度诊断" if deep else "轻量计时"}）',
        'echo 在相同地图和窗口大小下打开背包，等待 6 秒。',
        'echo 用相同顺序和节奏切换亚比 8 次，最后一次切换后等待 6 秒，再正常关闭游戏。',
        'echo 本轮记录保存至 "%run%"',
        '"%~dp0aola-native-test.exe" --aola-host --aola-data-dir "%run%"',
        'set "game_status=%errorlevel%"',
        f'python -B "%~dp0pet_switch_result.py" run "%run%" {mode} --game-exit "%game_status%"',
        'set "result_status=%errorlevel%"',
        'echo 本轮摘要与原始记录位于 "%run%"',
        'pause',
        'exit /b %result_status%',
    ])


def package(output, executable):
    output = output.resolve()
    if output.exists():
        raise ValueError(f"Choose a new package directory: {output}")
    binary = executable.read_bytes()
    for marker in (BUILD_ID, PERF_BUILD, "AOLA_DETAIL_PROFILE", "AOLA_GOTO_LEAF_FAST_PATH", "AOLA_ORPHAN_CENSUS"):
        if marker.encode("ascii") not in binary:
            raise ValueError(f"Required desktop component is absent: {marker}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as work:
        staging = Path(work) / "package"
        assets = staging / "swf-lifecycle"
        assets.mkdir(parents=True)
        util = ROOT / "resources/original" / f"library_util~{UTIL_VERSION}.swf"
        shutil.copyfile(util, assets / util.name)
        shutil.copyfile(REFERENCE, assets / "newpetpackage-original.swf")
        (staging / "aola-native-test.exe").write_bytes(binary)
        copy_licenses(staging)
        for name in ("capture_result.py", "pilot_resource.py", "pet_switch_result.py"):
            shutil.copyfile(Path(__file__).with_name(name), staging / name)
        for mode, label in (("A", "A-control"), ("B", "B-optimized"), ("deep", "deep-diagnostics")):
            (staging / f"run-{label}.cmd").write_bytes(launcher(mode))
        (staging / "compare-A-B.cmd").write_bytes(batch([
            "@echo off", "setlocal",
            'python -B "%~dp0pet_switch_result.py" compare --save-dir "%~dp0comparisons" %*',
            'set "result_status=%errorlevel%"', "pause", "exit /b %result_status%",
        ]))
        staging.rename(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--executable", type=Path, required=True)
    args = parser.parse_args()
    package(args.output, args.executable)
    print(args.output.resolve())
