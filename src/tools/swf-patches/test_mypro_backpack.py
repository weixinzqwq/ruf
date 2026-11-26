"""Exercise the packaged backpack revision guard with an offline asset domain and host."""
import struct

from observe_skin_motion import iter_tags, unpack_swf
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
    run_probe(args, "aola_backpack_revision", "MyProBackpackProbe", "MyProBackpackHost.java", b"",
              "initial false true\nrule changed true true\nsame rule false true\nrestore true true\n"
              "missing result false true\nmissing host false true\nhost resumed true true\n")
