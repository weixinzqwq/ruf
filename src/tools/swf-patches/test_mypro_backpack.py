"""Execute the packaged backpack resolver with an offline asset domain and host."""
import argparse
from pathlib import Path
import struct
import subprocess
from observe_skin_motion import ASC, ENGINE, INTERFACES, JAVA, TWEEN_LIBRARY, abc_tags, iter_tags, unpack_swf
from test_notice_patch import run_probe


if __name__=="__main__":
    ap=argparse.ArgumentParser(description=__doc__)
    for key in ("candidate","work","runner"): ap.add_argument("--"+key,type=Path,required=True)
    ap.add_argument("--parts",type=Path,help="Directory containing four original petmovie6044_N~1210.swf files")
    ap.add_argument("--poster",action="store_true",help="Exercise the optional backpack poster layer")
    ap.add_argument("--poster-asset",type=Path,help="Original petskinbackground1029 SWF for the cold-load test")
    args=ap.parse_args()
    body=unpack_swf(args.candidate.read_bytes())
    tags=list(iter_tags(body))
    abcs=[t for t in tags if t[0]==82]
    helper, = [t for t in abcs if t[2][4:].startswith(b"aola/tools/MyProBackpackMorph\0")]
    game, = [t for t in abcs if t[2][4:5]==b"\0"]
    assert b"MyProBackpackMorph" in game[2] and b"afterOtherOperate" in game[2]
    # Reuse the existing one-ABC fixture runner with the actual shipped helper.
    args.work=args.work.resolve()
    args.work.mkdir(parents=True)
    args.candidate=args.work/"helper.swf"
    for tag in reversed(abcs):
        if tag != helper:
            body=body[:tag[1]]+body[tag[1]+len(tag[3]):]
    args.candidate.write_bytes(b"FWS\x12"+struct.pack("<I",len(body)+8)+body)
    if args.poster:
        if args.poster_asset:
            packed=args.poster_asset.read_bytes()
            movie=unpack_swf(packed)
            if not any(k==76 and b"mmo.petskin.view.Background1029\0" in p for k,_,p,_ in iter_tags(movie)):
                raise ValueError("Expected the original Background1029 SymbolClass")
            art=args.work/"art.fws"
            art.write_bytes(b"FWS"+packed[3:8]+movie)
        else:
            art=args.work/"PosterArt.swf"
            source=art.with_suffix(".as")
            source.write_text('package {import flash.display.Sprite; public class PosterArt extends Sprite {}}'
                              'package mmo.petskin.view {import flash.display.Sprite;public class Background1029 extends Sprite {'
                              'public function Background1029(){graphics.beginFill(1);graphics.drawRect(0,0,5,5);}}}',encoding="ascii")
            globals_dir=ENGINE/"core/src/avm2/globals"
            subprocess.run([JAVA,"-classpath",ASC,"macromedia.asc.embedding.ScriptCompiler","-optimize","-builtin","-apiversioning",
                            "-outdir",args.work,"-out","artglobal",globals_dir/"Toplevel.as",globals_dir/"globals.as"],check=True)
            subprocess.run([JAVA,"-jar",ASC,"-AS3","-import",args.work/"artglobal.abc","-swf","PosterArt,10,10,30",source],check=True)
        run_probe(args,"aola_backpack_poster","MyProBackpackPosterProbe","MyProBackpackHost.java",b"",
                  "official original 0\nwarm Main false 1 17 55 7\nsame Main 2\nwide Wide -200 skin19 Variant\n"
                  "invalid original class-mismatch\nmissing original config-missing\npending original preload-pending cold\n"
                  "failed original preload-error\ncancelled original 1\nstale original 0\nhost off original\n"
                  "cold original pending\nloaded mmo.petskin.view::Background1029 true 23\n",frames=40,realtime=True,fixture_files=[(art,"art.swf")])
        raise SystemExit(0)
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
