"""Package the Aola desktop build and its notice, battle, and backpack resources."""
import argparse
import mmap
import shutil
import subprocess
import sys
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
BACKPACK_VERSION = "2026111953323155"
BACKPACK_ARCHIVED = ROOT / "resources/swf-reference/lifecycle" / BACKPACK_VERSION / f"newpetpackage~{BACKPACK_VERSION}.swf"
BACKPACK_SUPPLIED = ROOT / "newpetpackage.swf"


def write_launcher(path, *, background_zws=True, background_cws=True, direct_bitmap_groups=True,
                   hide_notice=False, block_skill=False, hide_transform_portrait=True,
                   skip_battle_backdrop=False, optimize_backpack=True, long_run=True, battle_cleanup=True,
                   close_phases=None, orphan_goto=None, target_goto=None, fuse_glow=None,
                   ordinary_orphans=None, branch_prune=False, battle_gc_catchup=False, battle_texture_pool=None,
                   pet_switch_timing=False, battle_glow_reuse=None, mypro_tools=None):
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
        'set "AOLA_SWF_OPERATIONS=1"',
        'if not defined AOLA_DISK_CACHE_BUDGET_MB set "AOLA_DISK_CACHE_BUDGET_MB=4096"',
        f'set "AOLA_BACKGROUND_ZWS={int(background_zws)}"',
        f'set "AOLA_BACKGROUND_CWS={int(background_cws)}"',
        f'set "AOLA_DIRECT_BITMAP_GROUPS={int(direct_bitmap_groups)}"',
        f'set "AOLA_HIDE_NOTICE={int(hide_notice)}"',
        f'set "AOLA_BLOCK_SKILL={int(block_skill)}"',
        f'set "AOLA_HIDE_TRANSFORM_PORTRAIT={int(hide_transform_portrait)}"',
        f'set "AOLA_SKIP_BATTLE_BACKDROP={int(skip_battle_backdrop)}"',
        f'set "AOLA_OPTIMIZE_BACKPACK={int(optimize_backpack)}"',
        f'set "AOLA_LONG_RUN_MAINTENANCE={int(long_run)}"',
        f'set "AOLA_BATTLE_CLEANUP={int(battle_cleanup)}"',
    ]
    if close_phases is not None:
        lines.append(f'set "AOLA_CLOSE_PHASES={int(close_phases)}"')
    if orphan_goto is not None:
        lines.append(f'set "AOLA_GOTO_ORPHAN_IDLE={int(orphan_goto)}"')
    if target_goto is not None:
        lines.append(f'set "AOLA_GOTO_TARGET_ONLY={target_goto}"')
    if fuse_glow is not None:
        lines.append(f'set "AOLA_FUSE_GLOW={int(fuse_glow)}"')
    if ordinary_orphans is not None:
        lines.append(f'set "AOLA_ORDINARY_ORPHANS={ordinary_orphans}"')
        lines.append(f'set "AOLA_ORDINARY_BRANCH_PRUNE={int(branch_prune)}"')
        lines.append(f'set "AOLA_BATTLE_GC_CATCHUP={int(battle_gc_catchup)}"')
    if battle_texture_pool is not None:
        lines.append(f'set "AOLA_BATTLE_TEXTURE_POOL={int(battle_texture_pool)}"')
    if pet_switch_timing:
        lines.append('set "AOLA_PET_SWITCH_TIMING=1"')
    if battle_glow_reuse is not None:
        lines.append(f'set "AOLA_BATTLE_GLOW_REUSE={int(battle_glow_reuse)}"')
    if mypro_tools is not None:
        lines.append(f'set "AOLA_MYPRO_TOOLS={int(mypro_tools)}"')
    rate_arg = " --frame-rate 60" if ordinary_orphans is not None else ""
    lines.extend([
        f'"%~dp0aola-native-test.exe" --aola-host{rate_arg} --aola-data-dir "%~dp0aola-data"',
        "exit /b %errorlevel%",
    ])
    path.write_bytes(("\r\n".join(lines) + "\r\n").encode("ascii"))


def package(output, executable, *, close_phases=False, orphan_goto=False, battle_render=False,
            battle_texture_pool=False, pet_switch_timing=False, battle_glow_reuse=False, mypro_tools=False):
    if mypro_tools:
        battle_glow_reuse = True
    if battle_glow_reuse:
        pet_switch_timing = True
    if pet_switch_timing:
        battle_texture_pool = True
    if battle_texture_pool:
        battle_render = True
    if battle_render:
        close_phases = orphan_goto = True
    output = output.resolve()
    if output.exists():
        raise FileExistsError(f"Choose a new output directory: {output}")
    resources = (("swf-notice/original.swf", NOTICE_ORIGINAL),
                 ("swf-battle/original.swf", BATTLE_ORIGINAL),
                 ("swf-backpack/archived.swf", BACKPACK_ARCHIVED),
                 ("swf-backpack/supplied.swf", BACKPACK_SUPPLIED))
    for _, original in resources:
        if not original.is_file():
            raise FileNotFoundError(f"Missing original game SWF: {original}")
    with executable.open("rb") as source, mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as image:
        for marker in (b"aolaHideNotice", b"aolaBlockSkill", b"AOLA_HIDE_TRANSFORM_PORTRAIT",
                       b"AOLA_DISK_CACHE_BUDGET_MB", b"AOLA_DXGI_CACHE_PRESSURE", b"AOLA_BACKGROUND_ZWS",
                       b"AOLA_BACKGROUND_CWS", b"AOLA_DIRECT_BITMAP_GROUPS", b"AOLA_SKIP_BATTLE_BACKDROP",
                       b"AOLA_OPTIMIZE_BACKPACK", b"AOLA_LONG_RUN_MAINTENANCE", b"AOLA_BATTLE_CLEANUP",
                       b"aola-weak-keys-battle-gc-v1"):
            if image.find(marker) < 0:
                raise ValueError(f"Desktop executable lacks {marker.decode()}: {executable}")
        if close_phases and image.find(b"AOLA_CLOSE_PHASES") < 0:
            raise ValueError(f"Desktop executable lacks AOLA_CLOSE_PHASES: {executable}")
        if orphan_goto and image.find(b"AOLA_GOTO_ORPHAN_IDLE") < 0:
            raise ValueError(f"Desktop executable lacks AOLA_GOTO_ORPHAN_IDLE: {executable}")
        if battle_texture_pool and image.find(b"AOLA_BATTLE_TEXTURE_POOL") < 0:
            raise ValueError(f"Desktop executable lacks AOLA_BATTLE_TEXTURE_POOL: {executable}")
        if pet_switch_timing and image.find(b"AOLA_PET_SWITCH_TIMING") < 0:
            raise ValueError(f"Desktop executable lacks AOLA_PET_SWITCH_TIMING: {executable}")
        if battle_glow_reuse and any(image.find(marker) < 0 for marker in
                                     (b"AOLA_BATTLE_GLOW_REUSE", b"glowCacheRebuilds")):
            raise ValueError(f"Desktop executable lacks glow cache diagnostics: {executable}")
        if mypro_tools and any(image.find(marker) < 0 for marker in
                               (b"AOLA_MYPRO_TOOLS", b"aolaMyProModelCandidate", b"morph.rules.replace")):
            raise ValueError(f"Desktop executable lacks the local tools and model rule support: {executable}")
        if battle_render:
            for marker in (b"AOLA_GOTO_TARGET_ONLY", b"AOLA_FUSE_GLOW", b"AOLA_ORDINARY_ORPHANS", b"AOLA_ORDINARY_BRANCH_PRUNE", b"AOLA_BATTLE_GC_CATCHUP"):
                if image.find(marker) < 0:
                    raise ValueError(f"Desktop executable lacks {marker.decode()}: {executable}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as work:
        staging = Path(work) / "package"
        for filename, original in resources:
            asset = staging / filename
            asset.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(original, asset)
        if mypro_tools:
            profiles = (
                ("battle", "patch_mypro_battle.py", ("swf-battle/mypro-model.swf",)),
                ("backpack", "patch_mypro_backpack.py", ("swf-backpack/supplied-mypro.swf", "swf-backpack/archived-mypro.swf")),
            )
            for name, tool, outputs in profiles:
                profile_dir = Path(work) / name
                subprocess.run([sys.executable, Path(__file__).with_name(tool),
                                "--output-dir", profile_dir], check=True)
                for relative in outputs:
                    shutil.copyfile(profile_dir / relative, staging / relative)
        shutil.copyfile(executable, staging / "aola-native-test.exe")
        shutil.copyfile(ROOT / "src/tools/compare-long-run.py", staging / "compare-long-run.py")
        if close_phases:
            shutil.copyfile(ROOT / "src/tools/measure-game-close.ps1", staging / "measure-game-close.ps1")
        copy_licenses(staging)
        launchers = (
            ("run-game.cmd", {}),
            ("run-previous-battle-cleanup.cmd", {"battle_cleanup": False}),
            ("run-original-maintenance.cmd", {"long_run": False}),
            ("run-original-backpack.cmd", {"optimize_backpack": False}),
            ("run-original-portraits.cmd", {"hide_transform_portrait": False}),
            ("run-control.cmd", {"background_zws": False}),
            ("run-foreground-cws.cmd", {"background_cws": False}),
            ("run-isolated-bitmap-groups.cmd", {"direct_bitmap_groups": False}),
            ("run-skip-battle-backdrop.cmd", {"skip_battle_backdrop": True}),
            ("run-hide-notices.cmd", {"hide_notice": True}),
            ("run-skill-fx-off.cmd", {"block_skill": True}),
            ("run-reduced-fx.cmd", {"hide_notice": True, "block_skill": True}),
        )
        if battle_texture_pool:
            common = dict(target_goto="battle", fuse_glow=True, ordinary_orphans="battle",
                          branch_prune=True, battle_gc_catchup=True)
            candidate = {**common, "battle_texture_pool": True, "pet_switch_timing": pet_switch_timing}
            launchers = [("run-game.cmd", candidate)]
            if battle_glow_reuse:
                launchers = [("run-game.cmd", {**candidate, "battle_glow_reuse": True}),
                             ("run-previous-rendering.cmd", {**candidate, "battle_glow_reuse": False})]
                if mypro_tools:
                    common_tools = {**candidate, "battle_glow_reuse": True}
                    launchers = [("run-game.cmd", {**common_tools, "mypro_tools": True}),
                                 ("run-player-only.cmd", {**common_tools, "mypro_tools": False})]
            elif not pet_switch_timing:
                launchers.append(("run-original-texture-pool.cmd", {**common, "battle_texture_pool": False}))
        elif battle_render:
            launchers = (
                ("run-game.cmd", {"target_goto": "battle", "fuse_glow": False, "ordinary_orphans": "battle", "branch_prune": True}),
                ("run-battle-gc-catchup.cmd", {"target_goto": "battle", "fuse_glow": False, "ordinary_orphans": "battle", "branch_prune": True, "battle_gc_catchup": True}),
                ("run-battle-gc-catchup-glow.cmd", {"target_goto": "battle", "fuse_glow": True, "ordinary_orphans": "battle", "branch_prune": True, "battle_gc_catchup": True}),
                ("run-sliced-battle-cleanup.cmd", {"target_goto": "battle", "fuse_glow": False, "ordinary_orphans": "battle", "branch_prune": True, "battle_cleanup": False}),
                ("run-original-ordinary-branches.cmd", {"target_goto": "battle", "fuse_glow": False, "ordinary_orphans": "battle"}),
                ("run-original-ordinary-orphans.cmd", {"target_goto": "battle", "fuse_glow": False, "ordinary_orphans": "0"}),
                ("run-goto-and-glow.cmd", {"target_goto": "battle", "fuse_glow": True, "ordinary_orphans": "battle", "branch_prune": True}),
                ("run-glow-only.cmd", {"target_goto": "0", "fuse_glow": True, "ordinary_orphans": "0"}),
                ("run-v2-control.cmd", {"target_goto": "0", "fuse_glow": False, "ordinary_orphans": "0"}),
            )
        for filename, options in launchers:
            log_close = close_phases and (battle_render or filename in ("run-game.cmd", "run-previous-battle-cleanup.cmd"))
            write_launcher(staging / filename, close_phases=log_close if close_phases else None,
                           orphan_goto=True if orphan_goto else None, **options)
        if orphan_goto and not battle_render:
            write_launcher(staging / "run-original-orphan-goto.cmd",
                           close_phases=True if close_phases else None, orphan_goto=False)
        staging.rename(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--close-phases", action="store_true",
                        help="Include the close measurement tool and enable phase logging in run-game.cmd.")
    parser.add_argument("--orphan-goto", action="store_true",
                        help="Enable idle orphan goto optimization and include a matching control launcher.")
    parser.add_argument("--battle-render", action="store_true",
                        help="Package the battle goto and glow candidates and their same-binary controls.")
    parser.add_argument("--battle-texture-pool", action="store_true",
                        help="Package the texture pool candidate and control with battle GC and glow fusion enabled.")
    parser.add_argument("--pet-switch-timing", action="store_true",
                        help="Record battle pet replacement phases on the current texture pool candidate.")
    parser.add_argument("--battle-glow-reuse", action="store_true",
                        help="Package the battle glow reuse candidate and its same-binary control.")
    parser.add_argument("--mypro-tools", action="store_true", help="Package the local tools panel on the glow reuse baseline.")
    args = parser.parse_args()
    package(args.output, args.executable, close_phases=args.close_phases, orphan_goto=args.orphan_goto,
            battle_render=args.battle_render, battle_texture_pool=args.battle_texture_pool,
            pet_switch_timing=args.pet_switch_timing, battle_glow_reuse=args.battle_glow_reuse, mypro_tools=args.mypro_tools)
    print(args.output.resolve())
