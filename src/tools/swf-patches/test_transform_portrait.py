"""Exercise the current battle movie's five transform portrait paths in headless AVM2."""
from observe_skin_motion import INTERFACES, abc_tags
from test_notice_patch import probe_args, run_probe

if __name__ == "__main__":
    channels = ["yiren", "jiuri", "juexing", "dengshen", "zhenli"]
    expected = "".join(f"{channel} 6 cases passed\n" for channel in channels)
    expected += "repeat yiren 2 cases passed\n"
    # The class loader needs its original base interface before loading ChangePetView.
    interface, = abc_tags(INTERFACES.read_bytes(), lambda name: name == b"mmo/interfaces/IDispose")
    run_probe(probe_args(__doc__), "aola_transform_portraits", "TransformPortraitProbe",
              "battle_probe.java", interface, expected, ("portraits",))
