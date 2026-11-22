"""Inventory public SWF evidence in this checkout. No network or game process is used."""

import argparse
import json
import re
import zipfile
from pathlib import Path, PureWindowsPath
from urllib.parse import urlsplit


ROOT = Path(__file__).resolve().parents[3]
ARCHIVE = Path("doc/archive/历史修改证据.zip")
PET_ROOT = "evidence/pet-detail-20261106/"
PET_RESOURCE = "/play/pet/newpetpackage~2026102936286521.swf"
PANEL_PATH = re.compile(
    r"/play/activityres/activity231229/petfightspace3/petfightspace3_[a-z0-9_]+~[0-9]+\.swf"
)


def public_resource(url):
    """Project catalogs only describe versioned SWFs at this exact origin."""
    parts = urlsplit(url)
    if parts.scheme not in ("http", "https") or parts.netloc != "aola.100bt.com":
        raise ValueError("unexpected origin in public resource catalog")
    if not re.fullmatch(r"/play/(?:[a-zA-Z0-9_-]+/)*[a-zA-Z0-9_-]+~[0-9]+\.swf", parts.path):
        raise ValueError("unexpected path in public resource catalog")
    return parts.path


def original_files(root):
    original = root / "resources/original"
    originals = sorted(p for p in original.iterdir() if p.suffix in (".swf", ".bin"))
    if not originals:
        raise ValueError("no primary SWF assets in resources/original")
    return {p.relative_to(root).as_posix(): p.stat().st_size for p in originals}


def catalog(root):
    rows = []
    for filename in ("startup-resources.json", "battle-resources.json"):
        path = Path("resources/metadata") / filename
        data = json.loads((root / path).read_text("utf-8-sig"))
        for entry in data["resources"] if isinstance(data, dict) else data:
            resource = public_resource(entry["url"])
            identity, version = resource[6:-4].rsplit("~", 1)
            if identity != entry["key"] or version != str(entry["version"]):
                raise ValueError("catalog key/version disagrees with URL")
            rows.append({"resource": resource, "version": version, "catalog": path.as_posix()})
    return rows


def embedded_socket(root):
    wrapper, = sorted((root / "resources/original").glob("library_socketserver1~*.bin"))
    inner = root / "resources/variants/socketserver-inner.swf"
    wrapped, body = wrapper.read_bytes(), inner.read_bytes()
    if len(wrapped) <= 8 or not body.startswith(b"CWS") or wrapped[8:] != body:
        raise ValueError("archived socket SWF does not match offset 8 of its wrapper")
    return {
        "wrapper": wrapper.relative_to(root).as_posix(), "offset": 8,
        "inner": inner.relative_to(root).as_posix(), "inner_bytes": len(body),
    }


def historical_downloads(root):
    downloads = []
    with zipfile.ZipFile(root / ARCHIVE) as archive:
        for manifest in ("acquisition.json", "modules.json"):
            metadata = PET_ROOT + "metadata/" + manifest
            for record in json.loads(archive.read(metadata)):
                if not urlsplit(record["url"]).path.endswith(".swf"):
                    continue
                resource = public_resource(record["url"])
                name = PureWindowsPath(record["file"]).name if manifest == "modules.json" else "versiondata.swf"
                member = PET_ROOT + "source/" + name
                size = archive.getinfo(member).file_size
                downloads.append({
                    "resource": resource, "manifest": metadata, "member": member,
                    "archived_bytes": size, "download_record_bytes": record.get("bytes"),
                })
    return downloads


def runtime_evidence(root):
    logs = sorted((root / "doc/evidence/runtime").rglob("startup-*.jsonl"))
    observations = []
    for path in logs:
        panels = {}
        hits = []
        with path.open(encoding="utf-8-sig") as stream:
            for lineno, line in enumerate(stream, 1):
                row = json.loads(line)
                if row.get("type") == "pet-swf-replacement" and type(row.get("optimized")) is bool:
                    optimized = row["optimized"]
                    hits.append({
                        "line": lineno, "mode": "local-patched" if optimized else "local-original",
                        "resource": PET_RESOURCE,
                    })
                if row.get("type") != "panel-diagnostic":
                    continue
                source, phase, value = row.get("source"), row.get("phase"), row.get("value")
                if not isinstance(source, str) or not PANEL_PATH.fullmatch(source):
                    continue
                if phase not in ("resource-request", "resource-status", "resource-body", "resource-body-error"):
                    continue
                if type(value) is not int or value < 0:
                    raise ValueError("invalid panel resource diagnostic")
                panels.setdefault(source, []).append({"line": lineno, "phase": phase, "value": value})
        if panels or hits:
            observations.append({
                "log": path.relative_to(root).as_posix(), "panel_events": panels, "pet_replacements": hits,
            })
    return {"startup_logs_scanned": len(logs), "observations": observations}


def inventory(root):
    page = root / "resources/original/play.html"
    entry_files = sorted(set(re.findall(r"\bstart~[0-9]+\.swf\b", page.read_text("utf-8-sig"))))
    return {
        "scope": "Local archive and historical logs only; no current server response measured.",
        "original_file_sizes": original_files(root),
        "static_entry_page": {"path": page.relative_to(root).as_posix(), "referenced_swf_names": entry_files},
        "catalog_entries": catalog(root),
        "embedded_socket_bytes": embedded_socket(root),
        "acquisition_archive": ARCHIVE.as_posix(),
        "historical_acquisition_records": historical_downloads(root),
        "historical_runtime": runtime_evidence(root),
        "interpretation": [
            "Catalog declarations, HTML references and contained bytes do not establish execution.",
            "Panel status and body length are historical observations. The archived log does not store response bytes.",
            "The historical pet hit event contains only the local mode; the resource path comes from aola_pet_patch.rs.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT, help="project checkout with local evidence")
    parser.add_argument("--output", type=Path, required=True, help="destination JSON file")
    args = parser.parse_args()
    report = inventory(args.root.resolve())
    output = args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote {output}")


if __name__ == "__main__":
    main()
