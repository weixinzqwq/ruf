"""Version and source of the backpack response used by the current skin pilot."""
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent.parent
VERSION = "2026111953323155"
UTIL_VERSION = "2026090340574444"
BUILD_ID = "swf-lifecycle-pilot-v2"
RESOURCE = "pet/newpetpackage"
FILENAME = f"newpetpackage~{VERSION}.swf"
REFERENCE = ROOT / "resources/swf-reference/lifecycle" / VERSION / FILENAME
