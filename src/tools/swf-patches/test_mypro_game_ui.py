"""Exercise the release start/control hooks across AVM2 domains and load all ten packaged skins."""
import argparse
import os
import shutil
import struct
import subprocess
from pathlib import Path

from mypro_game_ui import ARCHIVE, ASC, HERE, JAVA, compile_globals, compile_script
from observe_skin_motion import iter_tags, unpack_swf


def candidate_helper(source_movie, helper_name):
    helper, = (p for k, _, p, _ in iter_tags(unpack_swf(source_movie.read_bytes()))
                if k == 82 and p[4:].startswith(f"aola.tools.{helper_name}\0".encode()))
    return helper


def make_probe(work, fixture, cls, helper, dest):
    source=Path(shutil.copyfile(HERE/"harness"/f"{cls}.as",work/f"{cls}.as"))
    subprocess.run([JAVA,"-jar",ASC,"-AS3","-import",work/"playerglobal.abc","-swf",f"{cls},100,100,30",source],check=True)
    helper=helper.replace(b"flash.external",b"probe.external")
    game=unpack_swf(source.with_suffix(".swf").read_bytes())
    kind,pos,payload,raw=next(t for t in iter_tags(game) if t[0] in (72,82))
    def tag(p):
        return struct.pack("<HI",(82<<6)|63,len(p))+p
    probe=raw if kind==82 else tag(bytes(4)+b"probe\0"+payload)
    body=game[:pos]+probe+tag(helper)+game[pos+len(raw):]
    (fixture/dest).write_bytes(b"FWS\x12"+struct.pack("<I",len(body)+8)+body)


def run_probes(work, runner, skins, start=None, control=None):
    fixture=work/"tests/swfs/mypro_game_ui"
    fixture.mkdir(parents=True)
    compile_globals(work)
    for movie, cls, helper_name, dest in [
        (start, "MyProGameUiProbe", "MyProGameUi", "test.swf"),
        (control, "MyProGameUiControlProbe", "MyProGameUiDomain", "control.swf"),
    ]:
        helper = compile_script(work, helper_name) if movie is None else candidate_helper(movie, helper_name)
        make_probe(work, fixture, cls, helper, dest)
    for i in range(1,11):
        shutil.copyfile(skins/f"skin{i}.swf",fixture/f"skin{i}.swf")
    expected="""empty -1 unavailable
child only false 0
settings requested activityres/activity240927/controlbarskinsetting/controlbarskinsetting_code mmo.activity240927.controlbarskinsetting.panel.CBSS_MainPl
summoner requested activityres/activity250516/newzhaohuanshi/newzhaohuanshi_code mmo.activity250516.newzhaohuanshi.panel.NZHS_MainPl
unknown blocked
battle WaitingBattleEnd blocked
after Applied 10
native Applied 0
"""
    (fixture/"output.txt").write_text(expected+"".join(f"leaf {i} true true\n" for i in range(1,11)),encoding="ascii")
    (fixture/"test.toml").write_text("num_frames = 100\n",encoding="ascii")
    # A headless playback can finish 100 frames before the background decoder
    # completes the sequential skin loads. Test the AVM2 callbacks synchronously.
    env = {**os.environ, "RUFFLE_TEST_OPTS": "--compile-mode use-precompiled",
           "AOQI_STAGE3_MODE": "control", "AOLA_BACKGROUND_ZWS": "0", "AOLA_BACKGROUND_CWS": "0"}
    result = subprocess.run([runner, "mypro_game_ui", "--exact"], cwd=work, env=env,
                            capture_output=True, timeout=90)
    log=result.stdout+result.stderr
    (work/"runner.log").write_bytes(log)
    print(log.decode(errors="replace"))
    if result.returncode or b"1 passed" not in log:
        raise ValueError(f"Start/control callback and theme AVM2 regression failed. See {work / 'runner.log'}")


def test(bundle, work, runner):
    run_probes(work, runner, bundle, bundle / "start-ui.swf", bundle / "control-ui.swf")


def test_weekly(resource, candidate, runner, work):
    if resource not in ("start", "control"):
        raise ValueError(f"Unsupported weekly UI resource: {resource}")
    candidate = Path(candidate).resolve(strict=True)
    run_probes(Path(work).resolve(), Path(runner).resolve(strict=True),
               ARCHIVE / "05-retained/Themes/Live",
               start=candidate if resource == "start" else None,
               control=candidate if resource == "control" else None)


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ["bundle","work","runner"]:
        parser.add_argument("--"+key,type=Path,required=True)
    args=parser.parse_args()
    test(args.bundle.resolve(),args.work.resolve(),args.runner.resolve())
