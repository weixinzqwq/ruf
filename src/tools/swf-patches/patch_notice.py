"""Build the opt-in announcement patch for the archived messagesendservice."""
import argparse
from pathlib import Path
import re
import sys

from display_patch import ROOT, build, make_candidate

VERSION = "2026062579184482"
REFERENCE = ROOT / f"resources/swf-reference/notice/{VERSION}/messagesendservice~{VERSION}.swf"

if __name__ == "__main__":
    if "--weekly-source" in sys.argv:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--weekly-source", type=Path, required=True)
        parser.add_argument("--output-dir", type=Path, required=True)
        args = parser.parse_args()
        source = args.weekly_source.resolve(strict=True)
        if re.fullmatch(r"messagesendservice~[0-9]{1,32}\.swf", source.name) is None:
            parser.error("Expected official filename messagesendservice~<numeric version>.swf")
        work = args.output_dir.resolve()
        work.mkdir(parents=True)
        _, candidate = make_candidate(source, work, "notice_patch.java", source.stat().st_size)
        bundle = work / "swf-notice"
        bundle.mkdir()
        (bundle / "messagesendservice-notice.swf").write_bytes(candidate)
    else:
        build("patch_notice.py", VERSION, REFERENCE, 221474, "notice_patch.java",
              "aola_notice_edits.rs", "swf-notice", "messagesendservice-notice.swf")
