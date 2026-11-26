"""Build a version-locked lifecycle candidate from the captured backpack."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import struct
import subprocess
import textwrap
from pilot_resource import REFERENCE, RESOURCE, ROOT, VERSION
from roundtrip import JAVA, FFDEC_VERSION, JAVA_VERSION, jpexs, swf_body
from skin_recipe import rust_table


SKIN = "mmo.petpackage.controller.skin"
MANAGER = SKIN + ".PackageSkinManager"
BASE = SKIN + ".controller.MotionBase"
STYLE = SKIN + ".styles.Style"
METHODS = {
    MANAGER: {"joinNormals": 213, "joinSimple": 214, "init": 216, "changeClassify": 218, "dispose": 220},
    STYLE: {"dispose": 235},
    BASE: {"appear": 247, "dispose": 250},
}
for direction, index in (("Left", 267), ("Down", 272), ("Right", 277), ("Up", 282), ("Normal", 287)):
    METHODS[SKIN + ".controller.Motion" + direction] = {"disappear": index}


def public(name):
    return f'QName(PackageNamespace(""),"{name}")'


def motion(name):
    return f'QName(ProtectedNamespace("{SKIN}.controller:MotionBase"),"{name}")'


def private(num, name):
    return f'QName(PrivateNamespace(null,"{num}"),"{name}")'


STYLES = private(11, "_objs")
INSTANCE = private(11, "_instance")


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError(f"Expected one occurrence of {old!r}")
    return text.replace(old, new)


def cleanup(class_name, lookup):
    if class_name == MANAGER:
        # No new styles can be registered on this manager after disposal. A stale close
        # may arrive after a new singleton has been created.
        return f"""
            getlocal0
            getproperty {STYLES}
            setlocal1
            getlocal0
            pushnull
            setproperty {STYLES}
            getlocal1
            iffalse singleton
            pushbyte 0
            setlocal2
            jump more_styles
            next_style:
            label
            getlocal1
            getlocal2
            nextvalue
            callpropvoid {public("dispose")}, 0
            more_styles:
            hasnext2 1, 2
            iftrue next_style
            singleton:
            getlex {INSTANCE}
            getlocal0
            ifstrictne done
            findproperty {INSTANCE}
            pushnull
            setproperty {INSTANCE}
            done:
            returnvoid
        """, 3
    if class_name == STYLE:
        return f"""
            getlocal0
            getproperty {private(13, "_motion")}
            iffalse cleared
            getlocal0
            getproperty {private(13, "_motion")}
            callpropvoid QName(Namespace("mmo.interfaces:IDispose"),"dispose"), 0
            cleared:
            getlocal0
            pushnull
            setproperty {private(13, "_motion")}
            getlocal0
            pushnull
            setproperty {private(13, "_target")}
            getlocal0
            pushnull
            setproperty {private(13, "_disappearPos")}
            getlocal0
            pushnull
            setproperty {private(13, "_classifyArr")}
            returnvoid
        """, 1
    # This version of TweenLite keeps a list per target. The tag is the private
    # default-position Point allocated once by this Motion's constructor. It does
    # not refer back to the target. Never cancel another owner's tween on the target.
    return f"""
        getlocal0
        getproperty {motion("defaultPos")}
        iffalse cleared
        getlex QName(PackageNamespace("com.greensock"),"TweenLite")
        getproperty {public("masterList")}
        getlocal0
        getproperty {motion("target")}
        getproperty {lookup}
        setlocal1
        getlocal1
        iffalse cleared
        pushbyte 0
        setlocal2
        jump more_tweens
        next_tween:
        label
        getlocal1
        getlocal2
        nextvalue
        setlocal3
        getlocal3
        getproperty {public("data")}
        getlocal0
        getproperty {motion("defaultPos")}
        ifstrictne more_tweens
        getlocal3
        pushnull
        setproperty {public("data")}
        getlocal3
        getproperty {public("gc")}
        iftrue more_tweens
        getlocal3
        callpropvoid {public("kill")}, 0
        more_tweens:
        hasnext2 1, 2
        iftrue next_tween
        cleared:
        getlocal0
        pushnull
        setproperty {motion("defaultPos")}
        getlocal0
        pushnull
        setproperty {motion("disappearPos")}
        getlocal0
        pushnull
        setproperty {motion("target")}
        returnvoid
    """, 4


def patch_method(class_name, name, method, lookup):
    if name == "dispose":
        replacement, locals_count = cleanup(class_name, lookup)
        method = replace_once(method, "localcount 1", f"localcount {locals_count}")
        prefix, code = method.split("\ncode\n")
        _, suffix = code.split("\nend ; code\n")
        replacement = textwrap.dedent(replacement).strip()
        return f"{prefix}\ncode\ngetlocal0\npushscope\n{replacement}\nend ; code\n{suffix}"
    if class_name == MANAGER:
        guard = f"pushscope\ngetlocal0\ngetproperty {STYLES}\niftrue open_manager\nreturnvoid\nopen_manager:"
        return replace_once(method, "pushscope", guard)
    # Assign TweenCore.data on the returned tween after constructing it. The original
    # target, vars, duration and overwrite policy are exactly as in the source.
    call = public("to") + ", 3"
    return replace_once(method, "callpropvoid " + call,
                        f"callproperty {call}\ngetlocal0\ngetproperty {motion('defaultPos')}\nsetproperty {public('data')}")


def method_text(assembly, name):
    trait = rf'(?m)^\s*(?:override )?public function {name}\(.*?^\s+method\s*$(.*?)^\s+end ; method\s*$'
    matches = re.findall(trait, assembly, re.MULTILINE | re.DOTALL)
    if len(matches) != 1:
        raise ValueError(f"Could not uniquely identify {name}")
    return "\n".join(line.strip() for line in ("method" + matches[0] + "\nend ; method").splitlines()) + "\n"


def make_candidate(work, source):
    selector = ["-onerror", "abort", "-selectclass", SKIN + ".**"]
    version = jpexs(work, "coordinates", "-config", "showMethodBodyId=true", *selector,
                    "-export", "script", "as", source)
    if FFDEC_VERSION not in version:
        raise ValueError("Unexpected FFDec version")
    jpexs(work, "disassemble", *selector, "-format", "script:pcode", "-export", "script", "pcode", source)
    base_path = Path("pcode/scripts", *BASE.split(".")).with_suffix(".pcode")
    base = (work / base_path).read_text("utf-8-sig")
    # Reuse the original MotionBase dictionary/index lookup multiname.
    lookup, = re.findall(r"getproperty (MultinameL\([^\n]+\))", method_text(base, "appear"))
    args = []
    for class_name, methods in METHODS.items():
        class_path = Path("scripts", *class_name.split("."))
        as3 = (work / "as" / class_path.with_suffix(".as")).read_text("utf-8-sig")
        pcode = (work / "pcode" / class_path.with_suffix(".pcode")).read_text("utf-8-sig")
        for name, body in methods.items():
            coords = rf"function {name}\([^()]*\) : void\s*\{{\s*// method body index: {body} method index: \d+"
            if len(re.findall(coords, as3)) != 1:
                raise ValueError(f"Unexpected body for {class_name}.{name}")
            piece = patch_method(class_name, name, method_text(pcode, name), lookup)
            filename = f"body-{body}-{name}.pcode"
            (work / filename).write_bytes(piece.encode("utf-8"))
            args.extend([class_name, filename, str(body)])
    log = jpexs(work, "assemble", "-onerror", "abort", "-replace", source, "unverified.swf", *args)
    if log.count("Replace AS3 PCode") != len(args) // 3:
        raise ValueError("Not all methods were assembled")
    return work / "unverified.swf"


def split_movie(data):
    body = swf_body(data)
    pos = (5 + 4 * (body[0] >> 3) + 7) // 8 + 4
    # Compare physical tag headers too, apart from the expected DoABC length field.
    kept = [body[:pos]]
    abc = None
    while pos < len(body):
        start = pos
        kind, size = divmod(struct.unpack_from("<H", body, pos)[0], 64)
        pos += 2
        if size == 63:
            size, = struct.unpack_from("<I", body, pos)
            pos += 4
        end = pos + size
        if end > len(body):
            raise ValueError("Truncated SWF tag")
        if kind == 82:
            if abc is not None:
                raise ValueError("Unexpected second ABC")
            code = body.index(0, pos + 4, end) + 1
            kept.append((kind, body[pos:code]))
            abc = body[code:end]
        else:
            kept.append(body[start:end])
        pos = end
    if abc is None:
        raise ValueError("Missing DoABC")
    return kept, abc


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    work = (args.output_dir or ROOT / "build/swf-patches/pet-skin-lifecycle" /
            datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")).resolve()
    if ("(build " + JAVA_VERSION + ")").encode() not in subprocess.check_output(
        [JAVA, "-version"], stderr=subprocess.STDOUT
    ):
        raise ValueError("Unexpected Java version")
    reference = REFERENCE.read_bytes()
    work.mkdir(exist_ok=False, parents=True)
    source = work / "reference.swf"
    source.write_bytes(reference)
    candidate = make_candidate(work, source)
    original_tags, _ = split_movie(reference)
    patched_tags, abc = split_movie(candidate.read_bytes())
    if source.read_bytes() != reference or original_tags != patched_tags:
        raise ValueError("Source, movie header, non-ABC tags or DoABC identity changed")
    (work / "candidate.abc").write_bytes(abc)
    verifier = Path(__file__).with_name("skin_verify.java")
    ids = [str(i) for methods in METHODS.values() for i in methods.values()]
    with (work / "verify.log").open("wb") as log:
        subprocess.run([JAVA, "-cp", str(ROOT / "toolchain/ffdec/lib/*"), verifier,
                        source, candidate, work / "candidate.abc", *ids],
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    report = dict(resource=RESOURCE, version=VERSION, source=REFERENCE.relative_to(ROOT).as_posix(),
                  methods=METHODS, verified_top_level_tags=len(original_tags)-1,
                  abc_comparison="Only the declared code, local-register counts and approved appended names changed",
                  activation="explicit version-qualified pilot B only")
    recipe = rust_table(swf_body(reference), swf_body(candidate.read_bytes()))
    (work / "skin-recipe-table.rs").write_text(recipe, encoding="utf-8")
    recipe_module = ROOT / "src/optimization/engine/frontend-utils/src/backends/navigator/aola_skin_recipe.rs"
    if recipe not in recipe_module.read_text(encoding="utf-8"):
        raise ValueError("The rebuilt candidate does not match the reviewed desktop loader recipe")
    (work / "report.json").write_text(json.dumps(report, indent=2) + "\n", "utf-8")
    candidate.rename(work / "candidate.swf")
    print(work / "candidate.swf")


if __name__ == "__main__":
    main()
