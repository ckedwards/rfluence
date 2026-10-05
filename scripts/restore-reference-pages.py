#!/usr/bin/env python3
"""Recreate the reference pages from fixtures/confluence in a Confluence space.

For when the test site or space goes away, or the pages should live somewhere else.
See fixtures/confluence/README.md, "Restoring the pages".

  scripts/restore-reference-pages.py --space KEY [--parent ID] [--title-suffix S] [--dry-run] [name ...]

Without names, restores every fixture. Each page is created from its captured adf.json
(the latest version) and attachments/, in two passes so the pages can link to each other:
first all pages are created with their labels and attachments, then every body is written
with image IDs and links between the reference pages rewritten for the new site.

Credentials for the target site come from the environment, or from .env in the repo root.
The fixtures themselves are not changed.
"""
import argparse, base64, json, os, pathlib, re, sys, urllib.error, urllib.parse, urllib.request, uuid

ROOT = pathlib.Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures/confluence"


def env():
    vals = dict(os.environ)
    dotenv = ROOT / ".env"
    if dotenv.exists():
        for line in dotenv.read_text().splitlines():
            if "=" in line and not line.lstrip().startswith("#"):
                k, v = line.split("=", 1)
                vals.setdefault(k.strip(), v.strip().strip('"').strip("'"))
    for k in ("CONFLUENCE_EMAIL", "CONFLUENCE_API_KEY", "CONFLUENCE_BASE_URL"):
        if not vals.get(k):
            sys.exit(f"error: {k} is not set")
    return vals


class Site:
    def __init__(self, e):
        self.base = e["CONFLUENCE_BASE_URL"].rstrip("/")
        self.auth = "Basic " + base64.b64encode(f'{e["CONFLUENCE_EMAIL"]}:{e["CONFLUENCE_API_KEY"]}'.encode()).decode()

    def call(self, method, path, body=None, headers=None, raw=None):
        data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
        req = urllib.request.Request(self.base + path, data=data, method=method)
        req.add_header("Authorization", self.auth)
        req.add_header("Accept", "application/json")
        if body is not None:
            req.add_header("Content-Type", "application/json")
        for k, v in (headers or {}).items():
            req.add_header(k, v)
        try:
            with urllib.request.urlopen(req) as r:
                text = r.read()
                return json.loads(text) if text else None
        except urllib.error.HTTPError as err:
            sys.exit(f"error: {method} {path}: {err.code} {err.read().decode(errors='replace')[:500]}")

    def upload(self, page_id, path):
        boundary = uuid.uuid4().hex
        parts = [
            f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{path.name}"\r\n'
            f"Content-Type: application/octet-stream\r\n\r\n".encode() + path.read_bytes() + b"\r\n",
            f'--{boundary}\r\nContent-Disposition: form-data; name="minorEdit"\r\n\r\ntrue\r\n'.encode(),
            f"--{boundary}--\r\n".encode(),
        ]
        r = self.call(
            "POST", f"/wiki/rest/api/content/{page_id}/child/attachment", raw=b"".join(parts),
            headers={"X-Atlassian-Token": "no-check", "Content-Type": f"multipart/form-data; boundary={boundary}"},
        )
        return r["results"][0]["extensions"]["fileId"]


def load(name):
    d = FIXTURES / name
    page = json.loads((d / "page.json").read_text())
    adf = json.loads((d / "adf.json").read_text())
    atts = json.loads((d / "attachments.json").read_text())["results"] if (d / "attachments.json").exists() else []
    return d, page, adf, atts


def walk(node, f):
    f(node)
    for c in node.get("content", []):
        walk(c, f)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--space", required=True, help="target space key")
    ap.add_argument("--parent", help="parent page or folder ID (default: the space homepage)")
    ap.add_argument("--title-suffix", default="", help="appended to every title (titles are unique per space)")
    ap.add_argument("--dry-run", action="store_true", help="report what would happen; change nothing")
    ap.add_argument("names", nargs="*", help="fixtures to restore (default: all)")
    args = ap.parse_args()

    names = args.names or sorted(p.name for p in FIXTURES.iterdir() if (p / "page.json").exists())
    site = Site(env())
    space = (site.call("GET", "/wiki/api/v2/spaces?" + urllib.parse.urlencode({"keys": args.space}))["results"] or [None])[0]
    if not space:
        sys.exit(f"error: no space {args.space} on {site.base}")
    parent = args.parent or space["homepageId"]

    fixtures = {n: load(n) for n in names}
    notes = []

    # Pass 1: create the pages and upload attachments.
    new_ids, file_ids = {}, {}
    for name, (d, page, adf, atts) in fixtures.items():
        title = page["title"] + args.title_suffix
        if args.dry_run:
            new_ids[page["id"]] = f"<new {name}>"
            print(f"would create {title!r} with {len(atts)} attachments, {len(page.get('labels', {}).get('results', []))} labels")
            continue
        placeholder = {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Restoring..."}]}]}
        r = site.call("POST", "/wiki/api/v2/pages", {
            "spaceId": space["id"], "status": "current", "title": title, "parentId": parent,
            "body": {"representation": "atlas_doc_format", "value": json.dumps(placeholder)},
        })
        new_ids[page["id"]] = r["id"]
        labels = [{"prefix": "global", "name": l["name"]} for l in page.get("labels", {}).get("results", [])]
        if labels:
            site.call("POST", f"/wiki/rest/api/content/{r['id']}/label", labels)
        for a in atts:
            f = d / "attachments" / a["title"]
            if f.exists():
                file_ids[a["fileId"]] = site.upload(r["id"], f)
            else:
                notes.append(f"{name}: attachment file {a['title']} not captured; its images will be broken")
        print(f"created {title!r}: {r['id']}")

    # Pass 2: rewrite and write the bodies.
    old_bases = {page["_links"]["base"] for _, page, _, _ in fixtures.values()}
    page_url = re.compile(r"(https://[^/\s\"]+/wiki)/spaces/([^/\s\"]+)/pages/(\d+)(?:/[^#?\s\"]*)?")
    for name, (d, page, adf, atts) in fixtures.items():
        new_id = new_ids[page["id"]]
        found = {"annotation": 0, "mention": 0, "custom emoji": 0, "merfluence": 0}
        unresolved = set()

        def rewrite_url(m):
            base, _key, old = m.groups()
            if base in old_bases and old in new_ids:
                return f"{site.base}/wiki/spaces/{args.space}/pages/{new_ids[old]}"
            if base in old_bases:
                unresolved.add(old)
            return m.group(0)

        def rewrite_strings(v):
            if isinstance(v, str):
                return page_url.sub(rewrite_url, v)
            if isinstance(v, dict):
                return {k: rewrite_strings(x) for k, x in v.items()}
            if isinstance(v, list):
                return [rewrite_strings(x) for x in v]
            return v

        def fix(n):
            attrs = n.get("attrs") or {}
            for k in [k for k in attrs if k.startswith("__")]:
                del attrs[k]
            before = len(n.get("marks", []))
            n["marks"] = [m for m in n.get("marks", []) if m["type"] != "annotation"]
            found["annotation"] += before - len(n["marks"])
            if not n["marks"]:
                n.pop("marks")
            if n["type"] == "media" and attrs.get("type") == "file":
                attrs["id"] = file_ids.get(attrs["id"], attrs["id"])
                attrs["collection"] = f"contentId-{new_id}"
            if n["type"] == "mention":
                found["mention"] += 1
            if n["type"] == "emoji" and attrs.get("id") and attrs.get("text") == attrs.get("shortName"):
                found["custom emoji"] += 1
            if n["type"] in ("extension", "bodiedExtension", "inlineExtension"):
                params = attrs.get("parameters", {})
                params.get("macroParams", {}).pop("_parentId", None)
                params.get("macroMetadata", {}).pop("macroId", None)
                params.pop("embeddedMacroContext", None)
                if str(attrs.get("extensionKey", "")).endswith("/static/mermaid-diagram"):
                    found["merfluence"] += 1

        walk(adf, fix)
        adf = rewrite_strings(adf)
        for what, count in found.items():
            if count:
                notes.append({
                    "annotation": f"{name}: {count} inline comment anchors dropped (comments can't be moved)",
                    "mention": f"{name}: {count} mentions keep their account IDs; they show correctly only for accounts on the new site",
                    "custom emoji": f"{name}: {count} custom emoji are specific to the old site; re-add them by hand",
                    "merfluence": f"{name}: {count} merfluence diagrams need the merfluence app installed on the new site",
                }[what])
        if unresolved:
            notes.append(f"{name}: links to pages that weren't restored left pointing at the old site: {sorted(unresolved)}")
        if args.dry_run:
            continue
        current = site.call("GET", f"/wiki/api/v2/pages/{new_id}")
        site.call("PUT", f"/wiki/api/v2/pages/{new_id}", {
            "id": new_id, "status": "current", "title": current["title"],
            "version": {"number": current["version"]["number"] + 1, "message": f"Restored from fixtures/confluence/{name}"},
            "body": {"representation": "atlas_doc_format", "value": json.dumps(adf)},
        })
        print(f"wrote {name}: {site.base}/wiki/spaces/{args.space}/pages/{new_id}")

    for note in notes:
        print("note:", note)
    if not args.dry_run:
        print("\nOld -> new page IDs:")
        for old, new in new_ids.items():
            print(f"  {old} -> {new}")


main()
