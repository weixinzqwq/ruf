"""Build the opt-in Wanyu attack animation patch for the archived petfightnew."""
from display_patch import ROOT, build

VERSION = "261112849126071"
REFERENCE = ROOT / f"resources/original/petfightnew~{VERSION}.swf"

if __name__ == "__main__":
    build("patch_battle.py", VERSION, REFERENCE, 5377898, "battle_patch.java",
          "aola_battle_edits.rs", "swf-battle", "petfightnew-skill.swf")
