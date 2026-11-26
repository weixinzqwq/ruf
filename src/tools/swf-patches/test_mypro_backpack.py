"""Execute the packaged backpack resolver with an offline asset domain and host."""
import argparse
from pathlib import Path
import struct
from observe_skin_motion import INTERFACES, TWEEN_LIBRARY, abc_tags, iter_tags, unpack_swf
from test_notice_patch import run_probe


if __name__=="__main__":
    ap=argparse.ArgumentParser(description=__doc__)
    for key in ("candidate","work","runner"): ap.add_argument("--"+key,type=Path,required=True)
    ap.add_argument("--parts",type=Path,help="Directory containing four original petmovie6044_N~1210.swf files")
    args=ap.parse_args()
    body=unpack_swf(args.candidate.read_bytes())
    tags=list(iter_tags(body))
    abcs=[t for t in tags if t[0]==82]
    assert len(abcs)==2 and abcs[0][2][4:].startswith(b"aola/tools/MyProBackpackMorph\0")
    assert b"MyProBackpackMorph" in abcs[1][2] and b"afterOtherOperate" in abcs[1][2]
    # Reuse the existing one-ABC fixture runner with the actual shipped helper.
    args.work=args.work.resolve()
    args.work.mkdir(parents=True)
    args.candidate=args.work/"helper.swf"
    body=body[:abcs[1][1]]+body[abcs[1][1]+len(abcs[1][3]):]
    args.candidate.write_bytes(b"FWS\x12"+struct.pack("<I",len(body)+8)+body)
    if args.parts:
        label="aola_backpack_real6044"
        parts=[]
        for i in range(4):
            raw=(args.parts/f"petmovie6044_{i}~1210.swf").read_bytes()
            # The headless runner can be built without ZWS support. Keep all movie tags intact.
            local=args.work/f"part{i}.fws"
            local.write_bytes(b"FWS"+raw[3:8]+unpack_swf(raw))
            parts.append((local,f"part{i}.swf"))
        # Use the published game parser and resource naming routines for this target.
        classes={b"mmo/interfaces/pet/peticon/"+x for x in (b"PetIconType",b"PetIconConfig",b"IconResType")}
        tags=b"".join(abc_tags(INTERFACES.read_bytes(),classes.__contains__))
        tags+=b"".join(abc_tags(TWEEN_LIBRARY.read_bytes(),lambda n:n.startswith(b"mmo/common/bitmapmovie/") or n==b"com/adobe/utils/StringUtil"))
        expected="cold 5336 config-missing\nunrelated 5337\n"+"".join(f"progress loading {i} 4\n" for i in range(1,5))
        expected+="progress loaded 4 4\nready 6044 parts 4\n"
        run_probe(args,label,"MyProBackpack6044Probe","MyProBackpackHost.java",tags,expected,frames=960,fixture_files=parts,realtime=True)
        raise SystemExit(0)
    run_probe(args,"aola_backpack_model","MyProBackpackProbe","MyProBackpackHost.java",b"",
              "official 1\nready legacy 2\nwrong class 1\nreason class-mismatch\nready movie 4400\nwarm cache 4400 1\n"
              "cold partial 1 1\nwhile pending preload-pending 99\nprepared piece0,piece1,piece2,piece3 4401\ncold new 1\nstale skips 4\n"
              "fallback 1 pm-data-missing 1 unsupported-id\nhost off 1\n",frames=24)
