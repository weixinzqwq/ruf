"""Keep the precise source location and a small official p-code diff on weekly build failures."""

from difflib import get_close_matches, unified_diff
from pathlib import Path
import re
import subprocess

from observe_skin_motion import JAVA, ROOT
from pilot_resource import REFERENCE
from roundtrip import FFDEC


HERE = Path(__file__).resolve().parent
FRAME = re.compile(r"\bat ([\w.$]+)\.([\w$<>]+)\((\w+\.java):(\d+)\)")
FULL_METHOD = re.compile(r"\b((?:mmo|aola)(?:\.\w+)+)(?:::|\.)(\w+|<init>)")
WANYU = "mmo.petfightnew.common.util.GQWanyuZhichaoEffectHelper"
PET = "mmo.petfightnew.common.MCPetContainer"
LOADER = "mmo.petfightnew.common.manager.GQKillBigMovieLoader"
DETAIL = "mmo.petpackage.panel.inclusion.PetBigIconDetail"
CONTROL = "mmo.controlv2.controlbarskin.ControlBarSkinHandler"


def reference_for(resource):
    if resource == "battle":
        from patch_mypro_battle import active_version

        return ROOT / "resources/original" / f"petfightnew~{active_version()}.swf"
    if resource == "backpack":
        return REFERENCE
    if resource == "notice":
        from patch_notice import REFERENCE as notice

        return notice
    from mypro_game_ui import CONTROL as control, START

    return control if resource == "control" else START


def source_line(file, position):
    path = HERE / file
    if not path.is_file():
        return ""
    lines = path.read_text(encoding="utf-8").splitlines()
    if 0 < position <= len(lines):
        return lines[position - 1].strip()
    return ""


def failed_log(work, error):
    default = getattr(error, "log", None)
    # Some builders redirect their Java step into a deeper log before reporting
    # the top-level Python subprocess failure. Retain the innermost evidence.
    logs = sorted(work.rglob("*.log"), key=lambda path: len(path.parts), reverse=True)
    for path in logs:
        with path.open("rb") as file:
            file.seek(max(path.stat().st_size - 16_384, 0))
            text = file.read().decode("utf-8", errors="replace")
        if FRAME.search(text) or path == default:
            return path, text
    return default, str(error)


def locate(resource, text):
    frames = [(owner.rsplit(".", 1)[-1], method, file, int(line))
              for owner, method, file, line in FRAME.findall(text)]
    failure = next((line for line in text.splitlines() if "Exception" in line and "java." in line), "")
    if not failure:
        failure = next((line for line in reversed(text.splitlines()) if line.startswith("ValueError:")), "")
    target = next(((cls, method) for cls, method in FULL_METHOD.findall(failure)
                   if cls.rsplit(".", 1)[-1][:1].isupper()), None)
    locations = [(file, line, source_line(file, line)) for _, _, file, line in frames if (HERE / file).is_file()]
    for (_, method, file, line) in frames:
        at = source_line(file, line)
        if resource == "notice" and file == "notice_patch.java" and method == "main":
            match = re.search(r'patch\(abc, "(\w+)", "(\w+)", (\d+),', at)
            if match:
                target = ("mmo.MessageSend." + match[1], match[2])
        elif resource == "battle" and file == "battle_patch.java":
            if method == "patchWanyu":
                target = (WANYU, "playAttackMovie")
            elif method == "main":
                match = re.search(r'patchPortrait\(abc, "(\w+)"', at)
                if match:
                    target = ("mmo.petfightnew.common.fightcontroller.ChangePetView", match[1])
                match = re.search(r'patchBackdrop\(abc, "(\w+)"', at)
                if match:
                    target = ("mmo.petfightnew.common.startmovie." + match[1], "enterFight")
            elif method == "patchPetCleanup" and target is None:
                if "callbacks" in at or "Callback" in at:
                    target = (LOADER, "loadJumpTipsOnMovie")
                elif "dispose" in at:
                    target = (PET, "dispose")
        elif resource == "backpack" and file == "MyProBackpackPatch.java":
            if method == "revision":
                target = (DETAIL, "updatePetIcon")
            elif method == "scale":
                target = (DETAIL, "afterOtherOperate")
    if target is None and resource == "control" and "control" in text.lower():
        target = (CONTROL, "refreshUsingControlBarSkin")
    return target, locations, failure


def disassemble(source, directory, target):
    directory.mkdir()
    log = directory / "ffdec.log"
    cls, method = target
    output = directory / "pcode"
    command = [JAVA, "-Djava.awt.headless=true", "-jar", FFDEC, "-onerror", "abort",
               "-selectclass", cls, "-format", "script:pcode", "-export", "script", output, source]
    with log.open("wb") as stream:
        subprocess.run(command, cwd=directory, stdout=stream, stderr=subprocess.STDOUT, timeout=60)
    path = (output / "scripts" / Path(*cls.split("."))).with_suffix(".pcode")
    if not path.is_file():
        return None, []
    lines = path.read_text(encoding="utf-8-sig").splitlines()
    header = re.compile(r"\bfunction\s+(?:get\s+|set\s+)?(\w+)\s*\(")
    starts = {match[1]: position for position, line in enumerate(lines) if (match := header.search(line))}
    lookup = cls.rsplit(".", 1)[-1] if method == "<init>" else method
    closest = get_close_matches(lookup, starts, n=1, cutoff=0.8)
    if lookup not in starts and not closest:
        return str(path), []
    start = starts[lookup if lookup in starts else closest[0]]
    indent = len(lines[start]) - len(lines[start].lstrip())
    snippet = []
    for row in lines[start:]:
        snippet.append(row.strip())
        if row.strip() == "}" and len(row) - len(row.lstrip()) == indent:
            break
    return str(path), snippet


def failure_report(resource, version, official, work, error):
    log, text = failed_log(work, error)
    target, locations, failure = locate(resource, text)
    result = {"resource": resource, "version": version, "step": getattr(error, "step", "patch"),
              "script": locations[0][0] if locations else getattr(error, "script", None),
              "targetClass": target[0] if target else None, "targetMethod": target[1] if target else None,
              "expectedAnchor": failure or str(error), "message": str(error),
              "log": str(log) if log else None, "pcode": {}}
    if locations:
        result["sourceLocations"] = [{"script": file, "line": line, "statement": statement}
                                     for file, line, statement in locations]
    if target and official is not None:
        evidence = result["pcode"]
        try:
            reference = reference_for(resource)
            new_path, newer = disassemble(official, work / "official-pcode", target)
            old_path, older = disassemble(reference, work / "reference-pcode", target) if reference.is_file() else (None, [])
            evidence.update(officialFile=new_path, referenceFile=old_path, official=newer[:120],
                            expected=older[:120], difference=list(unified_diff(older, newer, "reference", "official", lineterm=""))[:160])
            if not newer:
                evidence["note"] = "Target class or method could not be exported from the new official movie"
        except (OSError, ValueError, subprocess.SubprocessError) as diagnostic_error:
            evidence["error"] = str(diagnostic_error)
    return result
