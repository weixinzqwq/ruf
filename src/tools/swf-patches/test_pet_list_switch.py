"""Compare the original and candidate PetList refresh on six slots in headless AVM2."""
import argparse
from argparse import Namespace
from pathlib import Path
import struct

from test_notice_patch import run_probe
from observe_skin_motion import INTERFACES, abc_tags


def measured_timeline():
    def tag(code, payload):
        return struct.pack("<HI", code << 6 | 63, len(payload)) + payload
    # Bind the counting MovieClip subclass to a real 50-frame timeline, so
    # currentFrame, totalFrames, play and stop all execute in Ruffle.
    sprite = struct.pack("<HH", 4000, 50) + struct.pack("<H", 1 << 6) * 50 + b"\x00\x00"
    interface, = abc_tags(INTERFACES.read_bytes(), lambda name: name == b"mmo/interfaces/IDispose")
    return interface + tag(39, sprite) + tag(76, struct.pack("<HH", 1, 4000) + b"probe.MeasuredClip\x00")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("original", "candidate", "runner", "work"):
        parser.add_argument("--" + key, type=Path, required=True)
    args = parser.parse_args()
    for key, calls in (("original", 18), ("candidate", 0)):
        trial = Namespace(candidate=getattr(args, key), runner=args.runner, work=args.work / key)
        run_probe(trial, "aola_list_switch", "PetListSwitchProbe", "pet_list_probe.java",
                  measured_timeline(), f"unchanged slot gotos {calls}\nselection hp reorder empty fullscreen passed\n")
