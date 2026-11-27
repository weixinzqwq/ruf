"""Reapply the reviewed local theme bridge to the current control-bar ABC."""
import re
import subprocess
from pathlib import Path

from roundtrip import FFDEC, JAVA, ROOT


HERE = Path(__file__).resolve().parent
HANDLER = "mmo.controlv2.controlbarskin.ControlBarSkinHandler"
SKIN = "mmo.controlv2.controlbarskin.ControlBarSkinBase"
LOCK = "mmo.controlv2.lock.ControlBarLockHelper"
CONTROL_CLASSES = (SKIN, *("mmo.controlv2." + name for name in (
    "HistoryPanelV2", "ActionPanel", "ChatControlHandler", "ControlBarBtnTipsHelper",
    "TalkBar", "EmotePanel", "BlueIcon", "RestGameTime", "RmbShoppingIcon",
    "player.PlayerInfo", "pause.PauseBtnMC", "pet.PetIcon", "phrase.PhrasePanel",
    "lock.ControlBarLockHelper", "buddybubble.BuddyBubbleTip",
    "galaxyfleet.GalaxyFleetTip", "squadbubble.SquadBubbleTip",
)))

METHOD = re.compile(r"(?m)^\s*(?:override\s+)?(?:public|private|protected)[^\n]*?function (\w+)\(")
LISTENER = re.compile(r'callpropvoid QName\([^\n]+,"(add|remove)EventListener"\), (\d+)')
THEME = f'QName(PackageNamespace("mmo.controlv2.controlbarskin"),"ControlBarSkinHandler")'


def run(command, log):
    with log.open("wb") as output:
        result = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=110)
    if result.returncode:
        raise ValueError(f"{HANDLER}.refreshUsingControlBarSkin: {log.name} failed, see {log}")


def ffdec(work, label, *args):
    run([JAVA, "-Djava.awt.headless=true", "-jar", FFDEC, "-onerror", "abort", *args],
        work / (label + ".log"))


def export(work, label, movie, owners, *, pcode=False):
    root = work / label
    args = ["-format", "script:pcode"] if pcode else []
    ffdec(work, label, "-selectclass", ",".join(owners), *args, "-export", "script", root, movie)
    return root / "scripts"


def exported(root, owner, extension):
    return root.joinpath(*owner.split(".")).with_suffix(extension)


def substitute_once(text, pattern, replacement, target):
    result, count = re.subn(pattern, replacement, text)
    if count != 1:
        raise ValueError(f"{target}: expected one {pattern}, found {count}")
    return result


def replace_once(text, expected, replacement, target):
    return substitute_once(text, re.escape(expected), lambda _: replacement, target)


def attach_theme(source):
    body = source.read_text(encoding="utf-8-sig")
    insertions = (
        ("import flash.display.MovieClip;", "import flash.display.MovieClip;\n   import flash.utils.getDefinitionByName;", "$iinit"),
        ("private function getCurrentSkin()", "private function getNativeSkin()", "getCurrentSkin"),
        ("this.getCurrentSkin().addToStage();", "setThemeListeners(this.getCurrentSkin(),true); this.getCurrentSkin().addToStage();", "addToStage"),
        ("this.removeFromStage();", "setThemeListeners(this.getCurrentSkin(),false); this.removeFromStage();", "refreshUsingControlBarSkin"),
        ("self._skinList.push({", "var nativeSkin:ControlBarSkinBase = new ControlBarSkinBase(self._usingSkinId,param1); if(self._aolaThemeId != 0) setThemeListeners(nativeSkin,false); self._skinList.push({", "initCurrentSkin"),
        ('"skin":new ControlBarSkinBase(self._usingSkinId,param1)', '"skin":nativeSkin', "initCurrentSkin"),
    )
    for old, new, member in insertions:
        body = replace_once(body, old, new, f"{HANDLER}.{member}")
    methods = (HERE / "ControlBarTheme.methods.as").read_text(encoding="utf-8-sig")
    close = body.rfind("   }")
    if close < 0:
        raise ValueError(f"{HANDLER}: class terminator missing")
    source.write_text(body[:close] + methods + body[close:], encoding="utf-8")


def method_pcode(source):
    text = source.read_text(encoding="utf-8-sig")
    names = list(METHOD.finditer(text))
    for i, name in enumerate(names):
        sink = names[i + 1].start() if i + 1 < len(names) else len(text)
        section = text[name.end():sink]
        block = re.search(r"(?ms)^\s*method\s*$.*?^\s*end ; method\s*$", section)
        if block:
            yield ("$iinit" if name[1] == source.stem else name[1]), block[0]


def handler_bindings(source):
    wanted = {"$iinit", "refreshUsingControlBarSkin"}
    for method, body in method_pcode(source):
        if method in wanted:
            wanted.remove(method)
            yield method, substitute_once(
                body, r'callproperty Multiname\("getUsingSkinId",[^\n]+\), 0',
                'callproperty QName(Namespace("mmo.interfaces.material.controlbarskin:IControlBarSkinService"),"getUsingSkinId"), 0',
                f"{HANDLER}.{method}")
    if wanted:
        raise ValueError(f"{HANDLER}: missing rebinding methods {sorted(wanted)}")


def track_listeners(source, owner):
    for member, body in method_pcode(source):
        owns_skin = owner == SKIN and member in ("$iinit", "addToStage")
        if not owns_skin and not LISTENER.search(body):
            continue
        target = f"{owner}.{member}"
        if owner == LOCK and member == "$iinit":
            assignment = re.search(r'getlocal0\s+getlocal1\s+initproperty [^\n]+"_controlBar"\)', body)
            if assignment is None:
                raise ValueError(f"{target}: missing _controlBar initialization")
            body = replace_once(body, assignment[0], "", target)
            body = replace_once(body, "constructsuper 0", "constructsuper 0\n" + assignment[0], target)

        variables = int(re.search(r"localcount (\d+)", body)[1])

        def call_tracker(match):
            action, count = match[1], int(match[2])
            if not 2 <= count <= 5:
                raise ValueError(f"{target}: unexpected EventDispatcher listener arity {count}")
            spill = [f"setlocal {variables + j}" for j in range(count, -1, -1)]
            load = [f"getlocal {variables + j}" for j in range(count + 1)]
            call = f'callpropvoid QName(PackageNamespace(""),"track{action.title()}Listener"), {count + 2}'
            return "\n".join([*spill, f"getlex {THEME}", "getlocal0", *load, call])

        body = LISTENER.sub(call_tracker, body)
        if owns_skin:
            enter = f'\ngetlex {THEME}\ngetlocal0\ncallpropvoid QName(PackageNamespace(""),"beginThemeOwner"), 1\n'
            leave = f'getlex {THEME}\ncallpropvoid QName(PackageNamespace(""),"endThemeOwner"), 0\n'
            if member == "$iinit":
                body = replace_once(body, "constructsuper 0", "constructsuper 0" + enter, target)
            else:
                body = substitute_once(body, r"(?m)^\s*code\s*$", lambda m: m[0] + enter, target)
                helper = 'QName(PackageNamespace("mmo.controlv2.lock"),"ControlBarLockHelper")'
                pattern = rf'findpropstrict {re.escape(helper)}\s+getlocal0\s+getproperty ([^\n]+)\s+constructprop {re.escape(helper)}, 1\s+pop'
                body = substitute_once(body, pattern, lambda m: f'getlex {THEME}\ngetlocal0\ngetlocal0\ngetproperty {m[1].strip()}\ncallpropvoid QName(PackageNamespace(""),"initThemeLock"), 2', target)
            body = re.sub(r"(?m)^(\s*)(returnvoid|returnvalue)\s*$", lambda m: leave + m[2], body)

        body = substitute_once(body, r"localcount \d+", f"localcount {variables + 7}", target)
        yield member, substitute_once(body, r"maxstack (\d+)", lambda m: f"maxstack {int(m[1]) + 3}", target)


def replace_bodies(work, label, movie, replacements):
    targets = work / (label + "-targets.txt")
    targets.write_text("\n".join(f"{owner}\t{member}" for owner, member, _ in replacements) + "\n", encoding="utf-8")
    body_log = work / (label + "-indices.log")
    run([JAVA, "-Djava.awt.headless=true", "-cp", str(ROOT / "toolchain/ffdec/lib/*"),
         HERE / "WeeklyControlMethods.java", movie, targets], body_log)
    indices = body_log.read_text(encoding="utf-8").splitlines()
    lines = []
    for (owner, member, body), index in zip(replacements, indices, strict=True):
        if not index.isdecimal():
            raise ValueError(f"{owner}.{member}: invalid FFDec method body index")
        path = work / f"{label}-{owner}-{member.removeprefix('$')}.pcode"
        path.write_text("\n".join(line.strip() for line in body.splitlines()) + "\n", encoding="utf-8")
        lines.extend((owner, str(path), index))
    arguments = work / (label + "-args.txt")
    arguments.write_text("\n".join(lines) + "\n", encoding="utf-8")
    result = work / (label + ".swf")
    ffdec(work, label, "-replace", movie, result, arguments)
    return result


def build_control_theme(official, work):
    work.mkdir(parents=True, exist_ok=False)
    source = exported(export(work, "export-handler", official, (HANDLER,)), HANDLER, ".as")
    attach_theme(source)
    handler = work / "compiled-handler.swf"
    ffdec(work, "compile-handler", "-replace", official, handler, HANDLER, source)
    binding = exported(export(work, "export-bindings", handler, (HANDLER,), pcode=True), HANDLER, ".pcode")
    bound = replace_bodies(work, "restore-bindings", handler,
                           [(HANDLER, member, body) for member, body in handler_bindings(binding)])

    scripts = export(work, "export-listeners", bound, CONTROL_CLASSES, pcode=True)
    replacements = []
    for owner in CONTROL_CLASSES:
        edits = list(track_listeners(exported(scripts, owner, ".pcode"), owner))
        if not edits:
            raise ValueError(f"{owner}: no reviewed listener or ownership anchor found")
        replacements.extend((owner, member, body) for member, body in edits)
    return replace_bodies(work, "patch-listeners", bound, replacements)
