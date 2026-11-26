"""Exercise the packaged backpack revision, visual skin and HD scaling with an offline host."""
import struct

from observe_skin_motion import INTERFACES, abc_tags, iter_tags, unpack_swf
from test_notice_patch import probe_args, run_probe


if __name__ == "__main__":
    args = probe_args(__doc__)
    body = unpack_swf(args.candidate.read_bytes())
    abcs = [tag for tag in iter_tags(body) if tag[0] == 82]
    helper, = [tag for tag in abcs if tag[2][4:].startswith(b"aola/tools/MyProBackpackMorph\0")]
    for tag in reversed(abcs):
        if tag != helper:
            body = body[:tag[1]] + body[tag[1] + len(tag[3]):]
    args.work = args.work.resolve()
    args.work.mkdir(parents=True)
    args.candidate = args.work / "helper.swf"
    args.candidate.write_bytes(b"FWS\x12" + struct.pack("<I", len(body) + 8) + body)
    scale_classes = {b"mmo/interfaces/pet/peticon/PetIconType", b"mmo/interfaces/pet/peticon/PetIconConfig"}
    native_scale = b"".join(abc_tags(INTERFACES.read_bytes(), scale_classes.__contains__))
    run_probe(args, "aola_backpack_revision", "MyProBackpackProbe", "MyProBackpackHost.java", native_scale,
              "initial false true\nrule changed true true\nsame rule false true\nrestore true true\n"
              "missing result false true\nmissing host false true\nhost resumed true true\n"
              "visual poster priority 5997 5336,533601,0\n"
              "visual invalid explicit poster 533601\n"
              "visual backpack skin 6044 5336,533601,0;5336,533601,6044\n"
              "visual missing skin record 533601 5336,533601,0\n"
              "visual confirmation changed 533601\nvisual rule restored 533601\nvisual absent host 533601 0\n"
              "visual refresh callback registered true\nvisual refresh updates active view 1 true\n"
              "visual refresh updates controls and icons 3 1 1 1 1 0\n"
              "visual refresh drops closed view 1 3 1 1 1 1 0\n"
              "visual background guards stale loads current,replacement\n"
              "visual normal voice handled locally true 6044,6044,6044\n"
              "visual mapped unlock stays local true 5997,5997\nvisual official unlock retained false 533601\n"
              "native scale groups true\n"
              "normal to .3 true true true true\nfresh attachment true true true true\n"
              ".3 to normal true true true true\n.3 to .4 true true true true\n"
              ".4 to .3 true true true true\nsame scale group true true true true\n"
              "no mapping true true true true\nbitmap ignores mapping true true true true\n"
              "unexpected hierarchy true true true true\nmissing host keeps scale true true true true\n")
