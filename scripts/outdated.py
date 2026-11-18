#!/usr/bin/env python3
"""List versions pinned in this repo that have newer upstream releases.

Covers what `cargo outdated` and `pnpm outdated` don't: container image tags
(Dockerfiles, workflows, compose files, shell scripts, and testcontainers
images in Rust tests), GitHub Actions versions, `node-version` in workflows,
the `packageManager` pnpm pin, and a numeric rust-toolchain channel.
`just outdated` runs it after those two tools.

Image tags are compared only with tags of the same shape: `node:26-alpine`
with other `<n>-alpine` tags, `postgres:18` with other `<n>` tags. Tags
without digits (`debian:trixie-slim`) can't be compared and are skipped, and
so are images under OWN_IMAGES. For node, only even (LTS) majors count.
Run with --all to also list what is already current.

The same file is used in each app repo; it finds everything from the repo's
tracked files, so it needs no per-repo configuration.
"""
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OWN_IMAGES = "ghcr.io/zetatwo/"
# testcontainers-modules images: `Postgres::default().with_tag("18")`.
MODULE_IMAGES = {"Postgres": "postgres", "Nats": "nats"}
REGISTRY_REF = re.compile(
    r"\b((?:docker\.io|ghcr\.io|quay\.io|registry\.k8s\.io|public\.ecr\.aws)/[\w./-]+:[\w][\w.-]*)")


def get(url, headers=None):
    req = urllib.request.Request(url, headers={"User-Agent": "outdated-check", **(headers or {})})
    with urllib.request.urlopen(req, timeout=30) as resp:
        return resp.read(), resp.headers


def get_json(url, headers=None):
    return json.loads(get(url, headers)[0])


def numbers(version):
    return tuple(int(n) for n in re.findall(r"\d+", version))


def change(current, latest):
    """'major'/'minor'/'patch' if latest is newer, compared at current's precision."""
    a = numbers(current)
    b = numbers(latest)[: len(a)]
    if b <= a:
        return None
    level = next(i for i, (x, y) in enumerate(zip(a, b)) if x != y)
    return ["major", "minor", "patch"][min(level, 2)]


# --- upstream lookups ------------------------------------------------------

def github_headers():
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        try:
            token = subprocess.run(["gh", "auth", "token"], capture_output=True,
                                   text=True, check=True).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            token = None
    return {"Authorization": f"Bearer {token}"} if token else {}


GITHUB_HEADERS = github_headers()


def github_latest(repo):
    return get_json(f"https://api.github.com/repos/{repo}/releases/latest", GITHUB_HEADERS)["tag_name"]


def registry_tags(image):
    """All tags of an image, via the OCI distribution API with anonymous auth."""
    first = image.split("/", 1)[0]
    if "." not in first and ":" not in first:  # docker.io shorthand
        image = "docker.io/" + (image if "/" in image else "library/" + image)
    registry, repo = image.split("/", 1)
    if registry == "docker.io":
        registry = "registry-1.docker.io"
    url = f"https://{registry}/v2/{repo}/tags/list?n=1000"
    headers, tags = {}, []
    while url:
        try:
            body, resp_headers = get(url, headers)
        except urllib.error.HTTPError as e:
            if e.code != 401 or headers:
                raise
            challenge = dict(re.findall(r'(\w+)="([^"]*)"', e.headers["WWW-Authenticate"]))
            query = urllib.parse.urlencode(
                {k: challenge[k] for k in ("service", "scope") if k in challenge})
            token = get_json(f"{challenge['realm']}?{query}")
            headers = {"Authorization": f"Bearer {token.get('token') or token['access_token']}"}
            continue
        tags += json.loads(body).get("tags") or []
        link = re.search(r"<([^>]+)>", resp_headers.get("Link", ""))
        url = urllib.parse.urljoin(url, link.group(1)) if link else None
    return tags


def image_latest(image, tag):
    shape = re.compile("^" + re.sub(r"\\\d+|\d+", r"\\d+", re.escape(tag)) + "$")
    tags = [t for t in registry_tags(image) if shape.match(t)]
    if image.rsplit("/", 1)[-1] == "node":
        tags = [t for t in tags if numbers(t)[0] % 2 == 0]
    return max(tags, key=numbers)


def node_lts_major():
    releases = get_json("https://nodejs.org/dist/index.json")
    return str(max(numbers(r["version"])[0] for r in releases if r["lts"]))


def npm_latest(package):
    return get_json(f"https://registry.npmjs.org/{package}/latest")["version"]


# --- pins in this repo -----------------------------------------------------

def tracked_files():
    out = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True,
                         text=True, check=True).stdout
    return [ROOT / p for p in out.splitlines()]


def images_in(path, text):
    """(image, tag) pairs referenced in one file."""
    name = path.name
    refs = []
    if name == "Dockerfile" or name.endswith(".Dockerfile"):
        refs += re.findall(r"^FROM\s+(?:--platform=\S+\s+)?(\S+)", text, re.M | re.I)
    if path.suffix in (".yml", ".yaml"):
        refs += re.findall(r"^\s*image:\s*[\"']?([^\s\"'#]+)", text, re.M)
    if path.suffix in (".yml", ".yaml", ".sh"):
        refs += REGISTRY_REF.findall(text)
        for line in text.splitlines():
            if re.search(r"\b(docker|podman|\"?\$\w+\"?)\s+run\b", line):
                refs += [t for t in line.split()
                         if re.fullmatch(r"[a-z][\w.-]*(/[\w.-]+)*:[\w][\w.-]*", t)]
    pairs = [tuple(r.rsplit(":", 1)) for r in refs
             if ":" in r.rsplit("/", 1)[-1] and "@" not in r]
    if path.suffix == ".rs":
        pairs += re.findall(r'GenericImage::new\(\s*"([^"]+)",\s*"([^"]+)"', text)
        pairs += [(MODULE_IMAGES[m], tag) for m, tag in
                  re.findall(r'\b(\w+)::default\(\)\s*\.with_tag\("([^"]+)"\)', text)
                  if m in MODULE_IMAGES]
    # `postgres` and `docker.io/library/postgres` are the same image; report it once.
    pairs = [(re.sub(r"^docker\.io/(library/)?", "", i), t) for i, t in pairs]
    return [(i, t) for i, t in pairs if not i.startswith(OWN_IMAGES) and re.search(r"\d", t)]


def pins():
    """Yield (kind, name, current, where, lookup) for everything pinned."""
    found = {}

    def add(kind, name, current, path, lookup):
        entry = found.setdefault((kind, name, current), [lookup, []])
        entry[1].append(str(path.relative_to(ROOT)))

    for path in tracked_files():
        if not path.is_file():
            continue
        try:
            text = path.read_text()
        except UnicodeDecodeError:
            continue
        for image, tag in images_in(path, text):
            add("image", image, tag, path, lambda image=image, tag=tag: image_latest(image, tag))
        if ".github/workflows" in path.as_posix():
            for repo, ref in re.findall(
                    r"uses:\s*([\w.-]+/[\w.-]+)(?:/[^@\s]*)?@(v?\d+(?:\.\d+)*)\b", text):
                add("action", repo, ref, path, lambda repo=repo: github_latest(repo))
            for version in re.findall(r"node-version:\s*[\"']?(\d+)", text):
                add("node", "node-version", version, path, node_lts_major)
        if path.name == "package.json":
            m = re.search(r'"packageManager":\s*"pnpm@([\d.]+)', text)
            if m:
                add("pnpm", "packageManager", m.group(1), path, lambda: npm_latest("pnpm"))
        if path.name == "rust-toolchain.toml":
            m = re.search(r'^channel\s*=\s*"(\d+\.\d+(?:\.\d+)?)"', text, re.M)
            if m:
                add("rust", "toolchain", m.group(1), path,
                    lambda: github_latest("rust-lang/rust"))

    for (kind, name, current), (lookup, paths) in found.items():
        where = paths[0] + (f" (+{len(paths) - 1} more)" if len(paths) > 1 else "")
        yield kind, name, current, where, lookup


def main():
    show_all = "--all" in sys.argv[1:]
    items = list(pins())

    def check(item):
        try:
            return item, item[4](), None
        except Exception as e:  # keep going; report per row
            return item, None, e

    with ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(check, items))

    rows, outdated, failed = [], 0, 0
    for (kind, name, current, where, _), latest, err in sorted(results, key=lambda r: r[0][:3]):
        if err:
            failed += 1
            rows.append((kind, name, current, "?", f"error: {err}", where))
            continue
        level = change(current, latest)
        if level:
            outdated += 1
        if level or show_all:
            rows.append((kind, name, current, latest, level or "ok", where))

    if rows:
        widths = [max(len(str(r[i])) for r in rows) for i in range(5)]
        for r in rows:
            print("  ".join(str(c).ljust(w) for c, w in zip(r, widths)) + "  " + r[5])
    print(f"\n{outdated} outdated, {failed} failed, {len(items)} checked")


if __name__ == "__main__":
    main()
