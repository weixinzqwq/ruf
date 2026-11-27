"""Build and atomically publish versioned Aola SWF patches for an existing player."""

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
from hashlib import sha256
import json
import lzma
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener
from xml.etree import ElementTree
import zlib

from observe_skin_motion import JAVA, iter_tags
from pilot_resource import REFERENCE, ROOT
from roundtrip import FFDEC


HERE = Path(__file__).resolve().parent
DEFAULT_ARCHIVE = ROOT / "resources/original"
BUILD = ROOT / "build"
RESOURCES = {
    "battle": "pet/petfightnew",
    "backpack": "pet/newpetpackage",
    "notice": "messagesend/messagesendservice",
    "control": "control/controlv2",
    "start": "start",
}
CDN = "https://aola.100bt.com/play/"
CATALOG_LIMIT = 32 * 1024 * 1024
MOVIE_LIMIT = 64 * 1024 * 1024
VERSION = re.compile(r"[0-9]{1,32}")
DIGEST = re.compile(r"[0-9a-f]{64}")
FILENAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,123}\.swf")
VERSION_SYMBOL = b"mmo.versiondata.VersionData_versionClass"


def version(value):
    if not isinstance(value, str) or not VERSION.fullmatch(value):
        raise ValueError(f"Unsupported official version {value!r}")
    return value


def official_path(resource):
    return "/play/" + RESOURCES[resource]


def fingerprint(data):
    return sha256(data).hexdigest()


class OfficialRedirects(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urlsplit(newurl) != urlsplit(req.full_url):
            raise HTTPError(req.full_url, code, "Unexpected official redirect", headers, fp)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def bounded_read(path, limit):
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError(f"File exceeds size limit: {path}")
    return data


class Source:
    def __init__(self, catalog_dir):
        self.catalog_dir = catalog_dir
        self.opener = build_opener(ProxyHandler({}), OfficialRedirects())

    def read(self, relative, limit):
        if self.catalog_dir is not None:
            return bounded_read(self.catalog_dir / Path(relative).name, limit)
        url = CDN + relative
        request = Request(url, headers={"Accept-Encoding": "identity", "User-Agent": "AolaRuffle-WeeklySync/1"})
        with self.opener.open(request, timeout=20) as response:
            if response.geturl() != url or response.status != 200:
                raise ValueError(f"Unexpected response fetching {url}")
            data = response.read(limit + 1)
        if len(data) > limit:
            raise ValueError(f"Response exceeds size limit: {url}")
        return data


def swf_tags(data, limit, *, fws=False):
    if len(data) < 14 or data[:3] not in (b"FWS", b"CWS", b"ZWS"):
        raise ValueError("Invalid SWF header")
    if fws and data[:3] != b"FWS":
        raise ValueError("Candidate must use FWS format")
    size = int.from_bytes(data[4:8], "little") - 8
    if not 6 <= size <= limit - 8:
        raise ValueError("Invalid SWF declared length")
    if data[:3] == b"FWS":
        body = data[8:]
    elif data[:3] == b"CWS":
        decoder = zlib.decompressobj()
        body = decoder.decompress(data[8:], size + 1)
        if not decoder.eof or decoder.unused_data:
            raise ValueError("Incomplete CWS stream or trailing data")
    else:
        if len(data) < 18 or struct.unpack_from("<I", data, 8)[0] != len(data) - 17:
            raise ValueError("Invalid ZWS compressed length")
        properties = data[12]
        dictionary = struct.unpack_from("<I", data, 13)[0]
        if properties >= 225 or not 4096 <= dictionary <= MOVIE_LIMIT:
            raise ValueError("Invalid ZWS decoder properties")
        filters = [{"id": lzma.FILTER_LZMA1, "lc": properties % 9,
                    "lp": properties // 9 % 5, "pb": properties // 45, "dict_size": dictionary}]
        decoder = lzma.LZMADecompressor(format=lzma.FORMAT_RAW, filters=filters)
        body = decoder.decompress(data[17:], max_length=size + 1)
        if decoder.unused_data:
            raise ValueError("Trailing ZWS data")
    if len(body) != size:
        raise ValueError("SWF length mismatch")
    tags = list(iter_tags(body))
    if not tags or tags[-1][0] != 0 or tags[-1][2] or any(tag[0] == 0 for tag in tags[:-1]):
        raise ValueError("Missing or misplaced SWF End tag")
    return tags


def catalog_xml(movie):
    symbols = []
    binaries = {}
    for kind, _, payload, _ in swf_tags(movie, CATALOG_LIMIT):
        if kind == 76:
            count, = struct.unpack_from("<H", payload)
            at = 2
            for _ in range(count):
                character, = struct.unpack_from("<H", payload, at)
                end = payload.index(0, at + 2)
                if payload[at + 2:end] == VERSION_SYMBOL:
                    symbols.append(character)
                at = end + 1
        elif kind == 87:
            character, = struct.unpack_from("<H", payload)
            if len(payload) < 6 or character in binaries:
                raise ValueError("Invalid version catalog binary tag")
            binaries[character] = payload[6:]
    if len(symbols) != 1 or symbols[0] not in binaries:
        raise ValueError("Version catalog has no unique VersionData_versionClass binding")
    return ElementTree.fromstring(binaries[symbols[0]])


def resource_versions(base, updates):
    if base.tag != "version":
        raise ValueError("Invalid version catalog root")
    paths = set(RESOURCES.values())
    resolved = {}
    for entries in (base.findall("f"), updates):
        seen = set()
        for item in entries:
            path = item.get("n")
            if path not in paths or item.get("rn"):
                continue
            number = version(item.get("v"))
            if path in seen and resolved[path] != number:
                raise ValueError(f"Ambiguous version for {path}")
            seen.add(path)
            resolved[path] = number
    return {name: resolved[path] for name, path in RESOURCES.items()}


def discover(source):
    startup = ElementTree.fromstring(source.read("start.xml", CATALOG_LIMIT))
    if startup.tag != "r":
        raise ValueError("Invalid official startup XML root")
    catalog = version(startup.findtext("v"))
    base = catalog_xml(source.read(f"versiondata~{catalog}.swf", CATALOG_LIMIT))
    resolved = resource_versions(base, startup.findall("u/f"))
    catalog_start = resolved["start"]
    # The bootstrap runs before the version catalog is loaded. Match the same
    # resource the desktop host requests, even when the catalog lists an alias.
    host = ROOT / "src/optimization/engine/desktop/src/aola_host.rs"
    match = re.search(r'pub const START_URL: &str = "https://aola\.100bt\.com/play/start~([0-9]+)\.swf";',
                      host.read_text(encoding="utf-8"))
    if match is None:
        raise ValueError("Cannot determine the startup SWF used by the desktop host")
    resolved["start"] = version(match[1])
    return catalog, catalog_start, resolved


def object_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate manifest field: {key}")
        result[key] = value
    return result


def load_manifest(path):
    if not path.exists():
        return None, {}
    raw = bounded_read(path, 128 * 1024)
    doc = json.loads(raw, object_pairs_hook=object_pairs)
    if not (type(doc["schemaVersion"]) is int and doc["schemaVersion"] == 1
            and isinstance(doc["generatedAt"], str) and isinstance(doc["entries"], list)):
        raise ValueError("Unsupported installed patch manifest")
    entries = {}
    for entry in doc["entries"]:
        resource = entry["resource"]
        version(entry["version"])
        if (resource in entries or entry["path"] != official_path(resource)
                or not FILENAME.fullmatch(entry["candidateFile"])
                or not DIGEST.fullmatch(entry["officialSha256"])
                or not DIGEST.fullmatch(entry["candidateSha256"])):
            raise ValueError(f"Invalid manifest entry for {resource!r}")
        entries[resource] = entry
    return raw, entries


def archive_source(source, resource, number, archive):
    leaf = Path(RESOURCES[resource]).name
    filename = f"{leaf}~{number}.swf"
    path = archive / filename
    if path.exists():
        swf_tags(bounded_read(path, MOVIE_LIMIT), MOVIE_LIMIT)
        return path
    historical = {
        "backpack": REFERENCE,
        "notice": ROOT / "resources/swf-reference/notice" / number / filename,
        "control": ROOT / "resources/swf-reference/02-originals/official" / filename,
        "start": ROOT / "resources/swf-reference/02-originals/official" / filename,
    }.get(resource)
    if (source.catalog_dir is None and historical is not None
            and historical.name == filename and historical.is_file()):
        data = bounded_read(historical, MOVIE_LIMIT)
    else:
        data = source.read(f"{RESOURCES[resource]}~{number}.swf", MOVIE_LIMIT)
    swf_tags(data, MOVIE_LIMIT)
    atomic_write(path, data, immutable=True)
    return path


def atomic_write(target, data, *, immutable=False):
    target.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=target.parent, prefix=".weekly-", delete=False) as stream:
        temporary = Path(stream.name)
        try:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        except BaseException:
            stream.close()
            temporary.unlink(missing_ok=True)
            raise
    try:
        if not immutable:
            os.replace(temporary, target)
        else:
            try:
                os.link(temporary, target)
            except FileExistsError:
                if target.is_symlink() or bounded_read(target, MOVIE_LIMIT) != data:
                    raise ValueError(f"Refusing to overwrite {target}") from None
    finally:
        temporary.unlink(missing_ok=True)


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


@contextmanager
def publication_lock(patches):
    patches.mkdir(parents=True, exist_ok=True)
    with (patches / ".weekly-sync.lock").open("a+b") as lock:
        lock.seek(0)
        if os.name == "nt":
            import msvcrt

            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl

            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield  # The file close releases the lock after success or failure.


class StepFailure(Exception):
    def __init__(self, step, script, log):
        super().__init__(f"{script} failed during {step}")
        self.step = step
        self.script = script
        self.log = log


def run_command(step, command, log, *, cwd=None, timeout=360, script=None):
    script = script or str(command[1])
    with log.open("wb") as stream:
        stream.write((subprocess.list2cmdline([str(item) for item in command]) + "\n").encode("utf-8"))
        stream.flush()
        try:
            result = subprocess.run(command, cwd=cwd, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=timeout)
        except (OSError, subprocess.TimeoutExpired) as error:
            stream.write(f"\n{error}\n".encode("utf-8"))
            raise StepFailure(step, script, log) from error
    if result.returncode:
        raise StepFailure(step, script, log)


def run_script(step, script, args, log):
    run_command(step, [sys.executable, HERE / script, *args], log)


def build_candidate(resource, official, work):
    if resource == "battle":
        script, output = "patch_mypro_battle.py", "swf-battle/mypro-model.swf"
    elif resource == "backpack":
        script, output = "patch_mypro_backpack.py", "swf-backpack/weekly-mypro.swf"
    elif resource == "notice":
        script, output = "patch_notice.py", "swf-notice/messagesendservice-notice.swf"
    else:
        out = work / "patch"
        run_script("patch", "mypro_game_ui.py",
                   ["--weekly-resource", resource, "--source", official, "--output-dir", out], work / "patch.log")
        return out / f"{resource}.fws.swf"
    out = work / "patch"
    run_script("patch", script, ["--weekly-source", official, "--output-dir", out], work / "patch.log")
    return out / output


REGRESSIONS = {
    "battle": ("test_mypro_battle_effects.py", "test_mypro_battle_background.py", "test_mypro_battle_auto.py"),
    "backpack": ("test_mypro_backpack.py", "test_mypro_lineup.py"),
    "notice": ("test_notice_patch.py",),
}


def qualify(resource, candidate, runner, work):
    data = bounded_read(candidate, MOVIE_LIMIT)
    swf_tags(data, MOVIE_LIMIT, fws=True)
    # FFDec must independently parse the emitted file. Builders also verify
    # their specific patched methods, or exact intact tags for startup helpers.
    run_command("readback", [JAVA, "-Djava.awt.headless=true", "-jar", FFDEC,
                            "-dumpSWF", candidate], work / "ffdec-readback.log", cwd=work,
                script="ffdec-cli.jar -dumpSWF")
    for script in REGRESSIONS.get(resource, ()):
        target = work / Path(script).stem
        run_script("regression", script, ["--candidate", candidate, "--runner", runner, "--work", target],
                   work / (target.name + ".log"))
    if resource in ("control", "start"):
        from mypro_game_ui import qualify_weekly_ui

        qualify_weekly_ui(resource, candidate, runner, work / "regression")
    return data


def find_runner(path, work):
    if path is None:
        builds = (BUILD / "optimization-target/release/deps").glob("tests-*.exe")
        candidates = sorted(builds, key=lambda file: file.stat().st_mtime, reverse=True)
    else:
        candidates = [path.resolve(strict=True)]
    probe = work / "runner-probe"
    fixture = probe / "tests/swfs/weekly_sync_runner_probe"
    fixture.mkdir(parents=True)
    body = b"\x08\x00\x00\x0c\x01\x00\x40\x00\x00\x00"
    (fixture / "test.swf").write_bytes(b"FWS\x12" + struct.pack("<I", len(body) + 8) + body)
    (fixture / "test.toml").write_text("num_frames = 1\n", encoding="ascii")
    (fixture / "output.txt").touch()
    with (probe / "runner.log").open("wb") as log:
        for candidate in candidates:
            result = subprocess.run([candidate, "weekly_sync_runner_probe", "--exact", "--list"],
                                    cwd=probe, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15)
            log.write(str(candidate).encode("utf-8") + b"\n" + result.stdout)
            if result.returncode == 0 and b"weekly_sync_runner_probe: test" in result.stdout:
                return candidate
    raise ValueError(f"Offline AVM2 runner not found. Pass --runner with the Ruffle tests executable; see {probe / 'runner.log'}")


def same_candidate(patches, entry):
    try:
        file = patches / entry["candidateFile"]
        if file.is_symlink() or not file.is_file():
            return False
        data = bounded_read(file, MOVIE_LIMIT)
        swf_tags(data, MOVIE_LIMIT, fws=True)
        return fingerprint(data) == entry["candidateSha256"]
    except (OSError, ValueError, struct.error):
        return False


def perform(args, source, report):
    patches = args.data_dir / "patches"
    manifest = patches / "manifest.json"
    report["manifest"] = str(manifest)
    before, installed = load_manifest(manifest)
    catalog, catalog_start, versions = discover(source)
    report.update(catalogVersion=catalog, catalogStartVersion=catalog_start)
    pending = {}
    for resource, number in versions.items():
        existing = installed.get(resource)
        state = "keep" if (existing and existing["version"] == number and same_candidate(patches, existing)) else "build"
        record = {"resource": resource, "path": official_path(resource), "version": number, "status": state}
        report["resources"].append(record)
        if state == "build":
            pending[resource] = record
    if args.dry_run:
        report["status"] = "dry-run"
        return
    if not pending and not args.update_embedded:
        report["status"] = "up-to-date"
        return
    # Publish all artifacts only after every changed component passes all gates.
    output = BUILD / "weekly-swfs"
    output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="sync-", dir=output))
    report["work"] = str(work)
    runner = find_runner(args.runner, work)
    artifacts = {}
    staged_profile = None
    for resource, record in pending.items():
        number = record["version"]
        case = work / resource
        case.mkdir()
        official = None
        try:
            official = archive_source(source, resource, number, args.originals_dir)
            candidate = build_candidate(resource, official, case)
            data = qualify(resource, candidate, runner, case)
            digest = fingerprint(data)
            name = f"{Path(RESOURCES[resource]).name}-{number}-{digest}.fws.swf"
            artifacts[name] = data
            installed[resource] = {"resource": resource, "path": record["path"], "version": number,
                                   "officialSha256": fingerprint(bounded_read(official, MOVIE_LIMIT)),
                                   "candidateFile": name, "candidateSha256": digest}
            record["status"] = "validated"
            if resource == "battle":
                staged_profile = case / "patch/aola_battle_profile.rs"
        except Exception as error:
            from weekly_diagnostics import failure_report

            record["status"] = "failed"
            report["failure"] = failure_report(resource, number, official, case, error)
            if official is None:
                report["failure"]["step"] = "archive"
            raise
    if args.update_embedded and staged_profile is None:
        case = work / "embedded-battle"
        case.mkdir()
        official = archive_source(source, "battle", versions["battle"], args.originals_dir)
        candidate = build_candidate("battle", official, case)
        qualify("battle", candidate, runner, case)
        staged_profile = case / "patch/aola_battle_profile.rs"
    with publication_lock(patches):
        if load_manifest(manifest)[0] != before:
            raise ValueError("Patch manifest changed while the new versions were being validated")
        for name, data in artifacts.items():
            atomic_write(patches / name, data, immutable=True)
        if args.update_embedded:
            from patch_mypro_battle import PROFILE

            old_profile = PROFILE.read_bytes()
            atomic_write(PROFILE, staged_profile.read_bytes())
        try:
            if pending:
                doc = {"schemaVersion": 1,
                       "generatedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                       "entries": [installed[resource] for resource in RESOURCES]}
                atomic_write(manifest, json_bytes(doc))
        except OSError:
            if args.update_embedded:
                atomic_write(PROFILE, old_profile)
            raise
        if args.update_embedded:
            report["embeddedUpdated"] = [str(PROFILE)]
    report["status"] = "published" if pending else "up-to-date"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true", help="Resolve versions without compiling or publishing")
    parser.add_argument("--data-dir", type=Path, default=ROOT / "dist/AolaRuffle/aola-data")
    parser.add_argument("--catalog-dir", type=Path, help="Use local start.xml, catalog and versioned SWFs")
    parser.add_argument("--originals-dir", type=Path, default=DEFAULT_ARCHIVE)
    parser.add_argument("--runner", type=Path, help="Ruffle regression tests executable")
    parser.add_argument("--report", type=Path, default=BUILD / "weekly-sync-report.json")
    parser.add_argument("--update-embedded", action="store_true", help="Activate the validated embedded battle profile")
    args = parser.parse_args()
    for name in ("data_dir", "catalog_dir", "originals_dir", "runner", "report"):
        path = getattr(args, name)
        if path is not None:
            setattr(args, name, path.resolve())
    if args.report == args.data_dir / "patches/manifest.json":
        parser.error("--report must differ from the player manifest")
    # Short synthetic movies complete before an asynchronous decoder can catch
    # up with synthetic frames. Run the offline callbacks with synchronous loads.
    os.environ.update(AOQI_STAGE3_MODE="sleep", AOLA_BACKGROUND_ZWS="0", AOLA_BACKGROUND_CWS="0")
    report = {"status": "failed", "resources": []}
    try:
        perform(args, Source(args.catalog_dir), report)
    except Exception as error:
        report.setdefault("failure", {"step": "sync", "message": str(error)})
        print(f"Weekly SWF sync failed: {error}", file=sys.stderr)
    atomic_write(args.report, json_bytes(report))
    print(f"{report['status']}: {args.report}")
    return int(report["status"] == "failed")


if __name__ == "__main__":
    raise SystemExit(main())
