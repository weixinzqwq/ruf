"""Build the opt-in announcement patch for the archived messagesendservice."""
from display_patch import ROOT, build

VERSION = "2026062579184482"
REFERENCE = ROOT / f"resources/swf-reference/notice/{VERSION}/messagesendservice~{VERSION}.swf"

if __name__ == "__main__":
    build("patch_notice.py", VERSION, REFERENCE, 221474, "notice_patch.java",
          "aola_notice_edits.rs", "swf-notice", "messagesendservice-notice.swf")
