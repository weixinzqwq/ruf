"""Exercise the production backpack helper with local replicas of the public game APIs."""
import argparse
import os
from pathlib import Path
import shutil
import struct
import subprocess
from mypro_lineup_backpack import HERE, TAG_NAME, abc_tag
from observe_skin_motion import ASC, ENGINE, JAVA, iter_tags, unpack_swf


def run(candidate, runner, work):
    fixture=work/"tests/swfs/mypro_lineup"
    fixture.mkdir(parents=True)
    source=Path(shutil.copyfile(HERE/"harness/MyProLineupProbe.as",work/"MyProLineupProbe.as"))
    globals_dir=ENGINE/"core/src/avm2/globals"
    subprocess.run([JAVA,"-classpath",ASC,"macromedia.asc.embedding.ScriptCompiler","-optimize","-builtin","-apiversioning",
                    "-outdir",work,"-out","playerglobal",globals_dir/"Toplevel.as",globals_dir/"globals.as"],check=True)
    subprocess.run([JAVA,"-jar",ASC,"-AS3","-import",work/"playerglobal.abc","-swf","MyProLineupProbe,100,100,30",source],check=True)
    helpers=[p for k,_,p,_ in iter_tags(unpack_swf(candidate.read_bytes())) if k==82 and p[4:].startswith(TAG_NAME)]
    helper,=helpers
    assert helper.count(b"flash.external")==1
    helper=helper.replace(b"flash.external",b"probe.external")
    body=unpack_swf(source.with_suffix(".swf").read_bytes())
    kind,start,payload,raw=next(t for t in iter_tags(body) if t[0] in (72,82))
    probe=raw if kind==82 else abc_tag(b"\0\0\0\0probe\0"+payload)
    body=body[:start]+abc_tag(helper)+probe+body[start+len(raw):]
    (fixture/"test.swf").write_bytes(b"FWS\x12"+struct.pack("<I",len(body)+8)+body)
    (fixture/"test.toml").write_text("num_frames = 3\n",encoding="ascii")
    (fixture/"output.txt").write_text("""initial 700 2 order,cards,artifacts,keys true 0
capture ready test 2
verified snapshot 111 true
order ready 222,111 1
same ready 1
cards ready 1 11
artifacts ready 1 98;37;x
keys ready 1 33
unbound failed 1
unowned failed 1
unowned artifact failed 1
clear key failed 1
owner failed 1
storage other owner failed 1
storage same owner ready 333,111 2
false ack failed 2
pending restoring
battle interrupts failed 1
closed true true
""",encoding="ascii")
    result=subprocess.run([runner,"mypro_lineup","--exact"],cwd=work,
                          env={**os.environ,"RUFFLE_TEST_OPTS":"--compile-mode use-precompiled"},capture_output=True,timeout=45)
    output=result.stdout+result.stderr
    (work/"runner.log").write_bytes(output)
    print(output.decode(errors="replace"))
    if result.returncode or b"1 passed" not in output:raise SystemExit(1)


if __name__=="__main__":
    ap=argparse.ArgumentParser(description=__doc__)
    for key in ("candidate","runner","work"):ap.add_argument("--"+key,type=Path,required=True)
    args=ap.parse_args()
    run(args.candidate.resolve(strict=True),args.runner.resolve(strict=True),args.work.resolve())
