"""Run MyPro v1.0.1 with its native performance logging on Windows."""
import argparse
from datetime import datetime, timezone
import json
import msvcrt
import os
from pathlib import Path
import re
import subprocess
import sys
import time
from uuid import uuid4


ROOT = Path(__file__).resolve().parents[2]
EXE = ROOT / "MyPro v1.0.1.exe"
PACKAGE = ROOT / "dist/mypro-battle-monitor-20261124"
EVENT = re.compile(
    rb"^\d{4}-\d\d-\d\dT\S+\s+INFO\s+(?:MyPro::(?:app|memory)|ruffle_core::player):\s+"
    rb"(?:MOVIE_RECLAIM(?:_SEC)?|HEAP_STAGE|HEAP_CENSUS|HEAP_CREATED|HEAP_TOP|HEAP_GROWTH|"
    rb"BATTLE_SCOPE(?:_RECLAIM|_ALIVE_TOP|_TAGGED_TOP)?|BATTLE_MEM|BATTLE_FOLLOWUP_MEM|"
    rb"ROOM_ENTRY_MEM|ROOM_ENTRY_FOLLOWUP_MEM|PANEL_RECLAIM_MEM)\b"
)


def utc():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


class LogTail:
    """Remember file identity on rename, and detect truncation or rewriting in place."""
    def __init__(self, data):
        self.data = data
        self.path = data / "MyPro.log"
        self.identity = None
        self.offset = 0
        self.previous = b""
        if self.path.is_file():
            with self.path.open("rb") as src:
                stat = os.fstat(src.fileno())
                self.identity = stat.st_dev, stat.st_ino
                self.offset = stat.st_size
                self.remember(src)

    def remember(self, src):
        start = max(0, self.offset - 96)
        src.seek(start)
        self.previous = src.read(self.offset - start)

    def continues(self, src):
        if os.fstat(src.fileno()).st_size < self.offset:
            return False
        src.seek(self.offset - len(self.previous))
        return src.read(len(self.previous)) == self.previous

    def drain(self, src, output):
        src.seek(self.offset)
        for line in src:
            if not line.endswith(b"\n"):
                break
            self.offset += len(line)
            if EVENT.match(line):
                output.write(line.rstrip(b"\r\n") + b"\n")
        self.remember(src)

    def poll(self, output):
        try:
            src = self.path.open("rb")
        except FileNotFoundError:
            return
        with src:
            stat = os.fstat(src.fileno())
            identity = stat.st_dev, stat.st_ino
            if self.identity is not None and (identity != self.identity or not self.continues(src)):
                # Drain unread records from a renamed segment, or its
                # copy-truncate backup, before consuming the newly active file.
                archives = sorted((p for p in self.data.glob("MyPro*.log*") if p != self.path and p.is_file()),
                                  key=lambda p: p.stat().st_mtime_ns, reverse=True)
                for old in archives:
                    with old.open("rb") as prior:
                        st = os.fstat(prior.fileno())
                        same_file = (st.st_dev, st.st_ino) == self.identity
                        if (same_file or self.previous) and self.continues(prior):
                            self.drain(prior, output)
                            break
                self.offset = 0
                self.previous = b""
            self.identity = identity
            self.drain(src, output)
        output.flush()


def check_native_lock(data):
    # MyPro uses its own .instance.lock for this data directory. Probe byte zero
    # (also covered by a whole-file lock); do not keep it while starting MyPro.
    with (data / ".instance.lock").open("a+b") as f:
        f.seek(0)
        try:
            msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
        except OSError as exc:
            raise RuntimeError("MyPro is already using this monitoring profile.") from exc
        f.seek(0)
        msvcrt.locking(f.fileno(), msvcrt.LK_UNLCK, 1)


def capture(options):
    offline = options.swf is not None
    if offline and (str(options.swf.resolve()).startswith("\\\\") or
                    not options.swf.is_file() or options.swf.suffix.lower() != ".swf"):
        raise ValueError("Choose an existing local .swf")
    if not offline and (options.seconds is not None or options.fake_battle is not None):
        raise ValueError("The timed fixture options require --swf")
    if options.seconds is not None and options.seconds <= 0:
        raise ValueError("--seconds must be positive")
    if not EXE.is_file():
        raise FileNotFoundError(EXE)
    base = PACKAGE if not offline else ROOT / "build/mypro-battle-monitor-20261124/offline-monitor"
    key = datetime.now().strftime("%Y%m%d-%H%M%S") + "-" + uuid4().hex[:8]
    run = base / "runs" / key
    data = (run if offline else base) / "data"
    data.mkdir(parents=True, exist_ok=True)
    with (data / ".performance-monitor.lock").open("a+b") as guard:
        guard.seek(0)
        try:
            msvcrt.locking(guard.fileno(), msvcrt.LK_NBLCK, 1)
        except OSError as exc:
            raise RuntimeError("A monitoring session already owns this profile.") from exc
        check_native_lock(data)
        run.mkdir(parents=True, exist_ok=True)
        cache = (run if offline else base) / "cache"
        cache.mkdir(exist_ok=True)
        env = os.environ.copy()
        env.update(
            AOQI_HEAP_CENSUS="1",
            RUST_LOG="warn,MyPro::app=info,MyPro::memory=info,ruffle_core::player=info",
        )
        env.pop("AOQI_FAST_GOTO", None)  # measure the shipping behavior
        args = [
            EXE,
            "--data-dir", data,
            "--cache-dir", cache,
            "--graphics", "dx",
            "--resolution", "960x560",
            "--mem-log", run / "mem.csv",
        ]
        if offline:
            args += ["--no-panel", "--swf", options.swf.resolve().as_uri()]
            env.update(
                HTTP_PROXY="http://127.0.0.1:9",
                HTTPS_PROXY="http://127.0.0.1:9",
                ALL_PROXY="http://127.0.0.1:9",
                NO_PROXY="",
                APPDATA=str(run),
                LOCALAPPDATA=str(run),
                WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--proxy-server=127.0.0.1:9 --disable-background-networking",
            )
            if options.seconds is not None:
                args += ["--exit-after-secs", str(options.seconds)]
            if options.fake_battle is not None:
                args += ["--fake-battle", options.fake_battle]
        tail = LogTail(data)
        session = {
            "launchUtc": utc(),
            "exe": str(EXE),
            "dataDir": str(data),
            "cacheDir": str(cache),
            "mode": "offline" if offline else "manual",
            "settings": {
                "graphics": "dx",
                "resolution": "960x560",
                "targetFps": None,
                "quality": "MyPro default",
                "heapCensus": 1,
                "fastGoto": "MyPro default",
            },
        }
        session_file = run / "session.json"

        def save():
            session_file.write_text(json.dumps(session, indent=2), encoding="utf-8")

        save()
        print(f"Run directory: {run}", flush=True)
        try:
            proc = subprocess.Popen(
                args,
                cwd=ROOT if not offline else run,
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            session["processId"] = proc.pid
            save()
            print(f"MyPro PID {proc.pid}. Close the game normally to finish the report.", flush=True)
            with (run / "telemetry.log").open("wb") as output:
                while proc.poll() is None:
                    tail.poll(output)
                    time.sleep(0.5)
                tail.poll(output)
            session["exitCode"] = proc.returncode
            session["endUtc"] = utc()
            save()
        except (OSError, KeyboardInterrupt) as exc:
            session["monitorError"] = type(exc).__name__ + ": " + str(exc)
            session["endUtc"] = utc()
            save()
            raise
        if proc.returncode:
            raise RuntimeError(f"MyPro exited with code {proc.returncode}. See {session_file}")
        if not (run / "mem.csv").is_file():
            raise RuntimeError(f"MyPro exited without opening its performance CSV. See {session_file}")
        subprocess.run(
            [sys.executable, Path(__file__).with_name("summarize-mypro-battle.py"), run, "--output", run / "summary.json"],
            check=True,
        )
        print(f"Report saved in {run}", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--swf", type=Path, help="Separate local fixture run with no game login")
    parser.add_argument("--seconds", type=int, help="Offline fixture time limit passed to MyPro")
    parser.add_argument("--fake-battle", help="Offline fixture only, start:end seconds")
    try:
        capture(parser.parse_args())
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
