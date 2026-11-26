#!/usr/bin/env python3
"""Write the third-party licence notices the Mac App Store bundle carries.

The Store edition's bundle distributes code the full edition never ships: the
DuckDB library Meridian builds, with its extensions compiled in, the C
libraries those extensions link, GEOS as a library of its own, and FineType's
Rust crates. Each licence asks that its text, and its copyright notices, go
with the code. This script writes them into one directory, which
scripts/package-store.sh copies to Contents/Resources/licenses/.

  notices.py managed OUT --duckdb-src DIR --duckdb-build DIR --lib-dir DIR
                         --finetype-src DIR --deny-config FILE
      what scripts/build-duckdb.sh built: the C libraries vcpkg built, GEOS's
      licence and where its source is, DuckDB's licence and those of the code
      it and its extensions carry, FineType's licence, and the Rust crates
      compiled into the library and into FineType's command line, after
      cargo-deny holds those crates to Brightfield's own licence policy.

  notices.py rust OUT_FILE --manifest-path FILE --package NAME [--features F]
                           --title TEXT --home URL
      the Rust crates one package of a workspace compiles in, with each
      crate's licence files, or, for a crate whose package carries none, the
      SPDX text of each licence its licence expression names.

  notices.py index DIR
      write DIR/README.txt, which says what the directory holds.

It lives under scripts/duckdb-managed/ because the managed build calls it, so
a change to it, or to the licence texts beside it, rebuilds that build and its
notices with it (xtask/src/duckdb_stamp.rs).
"""

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from typing import NoReturn

HERE = os.path.dirname(os.path.abspath(__file__))
SPDX_DIR = os.path.join(HERE, "spdx")
TARGET = "aarch64-apple-darwin"

# The vcpkg ports the managed build installs, by what happens to them. A port
# that ships is compiled into libduckdb.dylib, or, for GEOS, beside it; a
# build tool runs only while the build does. A port in neither list stops the
# build, so a library whose licence nobody has read cannot reach the bundle.
SHIPPED_PORTS = {
    "curl": "curl",
    "expat": "MIT",
    "gdal": "MIT",
    "geos": "LGPL-2.1-only",
    "libjpeg-turbo": "IJG AND BSD-3-Clause",
    "liblzma": "0BSD",
    "minizip-ng": "Zlib",
    "nlohmann-json": "MIT",
    "openssl": "Apache-2.0",
    "proj": "MIT",
    "roaring": "Apache-2.0 OR MIT",
    "sqlite3": "blessing (public domain)",
    "tiff": "libtiff",
    "zlib": "Zlib",
}
BUILD_TOOL_PORTS = {
    "pkgconf",
    "vcpkg-cmake",
    "vcpkg-cmake-config",
    "vcpkg-cmake-get-vars",
    "vcpkg-pkgconfig-get-modules",
    "vcpkg-tool-meson",
}

# Directories of DuckDB's source whose code the library does not carry: tests,
# benchmarks and their data, the TPC generators, extensions this build does
# not load, the build tree, and the Swift package.
DUCKDB_SKIP_DIRS = {
    ".git", "build", "test", "tests", "benchmark", "data", "catch", "imdb",
    "tpch", "tpcds", "delta", "demo_capi", "vcpkg_ports", "swift",
}
LICENCE_FILE = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)([.\-_].*)?$", re.I)


def fail(message) -> NoReturn:
    sys.exit(f"notices: {message}")


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


# --- vcpkg ---------------------------------------------------------------------


def installed_ports(installed):
    """The ports vcpkg installed, {name: version}, from its install database.

    vcpkg keeps that database as vcpkg/status plus a record per change in
    vcpkg/updates/, and folds the records into status only when a later run
    loads it. So, as vcpkg does, status is read first and then each update in
    name order, a later paragraph for a package replacing an earlier one."""
    database = os.path.join(installed, "vcpkg")
    status = os.path.join(database, "status")
    if not os.path.isfile(status):
        fail(f"no vcpkg status database at {status}")
    files = [status]
    updates = os.path.join(database, "updates")
    if os.path.isdir(updates):
        files += [os.path.join(updates, f) for f in sorted(os.listdir(updates))]
    entries = {}
    for path in files:
        for block in read(path).split("\n\n"):
            fields = {}
            for line in block.splitlines():
                key, _, value = line.partition(": ")
                fields[key] = value
            if "Package" not in fields:
                continue
            key = (fields["Package"], fields.get("Architecture"), fields.get("Feature"))
            entries[key] = fields
    ports = {}
    for (name, _, feature), fields in entries.items():
        if feature is not None or fields.get("Status") != "install ok installed":
            continue
        version = fields.get("Version", "")
        if fields.get("Port-Version", "0") != "0":
            version += "#" + fields["Port-Version"]
        ports[name] = version
    if not ports:
        fail(f"vcpkg's database in {database} lists no installed port")
    return ports


def c_libraries(out, triplet_dir, ports):
    for name in sorted(ports):
        if name in BUILD_TOOL_PORTS or name == "geos":
            continue
        if name not in SHIPPED_PORTS:
            fail(
                f"vcpkg installed the port {name}, which this script neither ships nor knows as a "
                "build tool: read its licence, then add it to SHIPPED_PORTS or BUILD_TOOL_PORTS"
            )
        copyright_file = os.path.join(triplet_dir, "share", name, "copyright")
        if not os.path.isfile(copyright_file):
            fail(f"vcpkg's port {name} left no licence file at {copyright_file}")
        header = (
            f"{name} {ports[name]}, compiled into libduckdb.dylib\n"
            f"Licence: {SHIPPED_PORTS[name]}\n"
            f"The text below is the licence file vcpkg's {name} port installs.\n\n"
        )
        write(os.path.join(out, "c-libraries", f"{name}.txt"), header + read(copyright_file))
    missing = sorted(set(SHIPPED_PORTS) - set(ports))
    if missing:
        fail(f"vcpkg did not install {', '.join(missing)}; take them out of SHIPPED_PORTS if the build no longer uses them")


# --- GEOS ----------------------------------------------------------------------


def cmake_arg(portfile, key):
    match = re.search(rf"\b{key}\s+\"?([^\s\")]+)\"?", portfile)
    return match.group(1) if match else None


def extension_commit(extensions_cmake, name):
    match = re.search(
        rf"duckdb_extension_load\({name}\b[^)]*?GIT_URL\s+(\S+)[^)]*?GIT_TAG\s+([0-9a-f]{{40}})",
        read(extensions_cmake),
        re.S,
    )
    if match is None:
        fail(f"no GIT_URL and GIT_TAG for {name} in {extensions_cmake}")
    return match.group(1), match.group(2)


def geos(out, duckdb_src, triplet_dir, lib_dir, ports, extensions_cmake, vcpkg_commit):
    """GEOS's licence, where its source is, and Meridian's offer of a copy.

    GEOS is LGPL-2.1, the one copyleft library the bundle carries. Hugh ruled
    on 2026-11-26 that the Store build ships it as it is built now, as its own
    replaceable library, with its licence text and an offer of its source.
    """
    if "geos" not in ports:
        fail("vcpkg installed no GEOS, which spatial needs")
    # The port spatial overlays on vcpkg's, which therefore is the one built.
    port_dir = os.path.join(duckdb_src, "extension", "external", "spatial", "vcpkg_ports", "geos")
    portfile_path = os.path.join(port_dir, "portfile.cmake")
    if not os.path.isfile(portfile_path):
        fail(f"no GEOS port at {port_dir}: the source offer below names spatial's overlay port, so read which port the build used and say so")
    port_version = json.loads(read(os.path.join(port_dir, "vcpkg.json")))["version"]
    if port_version != ports["geos"].split("#")[0]:
        fail(f"vcpkg installed GEOS {ports['geos']}, but spatial's overlay port is {port_version}")
    portfile = read(portfile_path)
    repo, ref = cmake_arg(portfile, "REPO"), cmake_arg(portfile, "REF")
    if repo != "libgeos/geos" or not ref or not re.fullmatch(r"[0-9a-f]{40}", ref):
        fail(f"spatial's GEOS port does not fetch libgeos/geos at a commit ({repo} {ref}); rewrite the source offer for it")
    patches_match = re.search(r"PATCHES((?:\s+[\w.\-]+\.patch)+)", portfile)
    patches = patches_match.group(1).split() if patches_match else []
    spatial_url, spatial_commit = extension_commit(extensions_cmake, "spatial")

    dest = os.path.join(out, "GEOS")
    copying = read(os.path.join(triplet_dir, "share", "geos", "copyright"))
    if "GNU LESSER GENERAL PUBLIC LICENSE" not in copying or "Version 2.1" not in copying:
        fail("GEOS's licence file is not the LGPL version 2.1; read it and rewrite this notice")
    write(os.path.join(dest, "COPYING"), copying)
    for patch in patches:
        source = os.path.join(port_dir, patch)
        if not os.path.isfile(source):
            fail(f"spatial's GEOS port names a patch it does not hold: {patch}")
        os.makedirs(os.path.join(dest, "patches"), exist_ok=True)
        shutil.copyfile(source, os.path.join(dest, "patches", patch))

    libraries = sorted(f for f in os.listdir(lib_dir) if f.startswith("libgeos") and f.endswith(".dylib"))
    if not libraries:
        fail(f"no GEOS library in {lib_dir}")
    patch_lines = (
        "".join(f"  patches/{p}\n" for p in patches)
        if patches
        else "  (none)\n"
    )
    write(
        os.path.join(dest, "SOURCE.txt"),
        f"""GEOS, the Geometry Engine Open Source
=====================================

This app carries GEOS, which DuckDB's spatial extension and GDAL use, as two
shared libraries of its own:

{"".join(f"  Contents/Frameworks/{lib}{chr(10)}" for lib in libraries)}
GEOS is free software, licensed under the GNU Lesser General Public License,
version 2.1 (LGPL-2.1). Its text is in COPYING, beside this file. The rest of
the app is not covered by that licence: it uses GEOS only through these
libraries, which you may replace with a GEOS of your own build. A replacement
has to keep GEOS's C API and the function geos_d2sfixed_buffered_n, which
DuckDB's spatial extension calls from GEOS's C++ library.

WHERE ITS SOURCE IS

The libraries were built from GEOS's source at commit

  https://github.com/libgeos/geos/tree/{ref}

with these changes applied, which DuckDB's spatial extension carries in its
GEOS port ({spatial_url}/tree/{spatial_commit}/vcpkg_ports/geos):

{patch_lines}
The patches are in this directory too. vcpkg
(https://github.com/microsoft/vcpkg at commit {vcpkg_commit}) built them, as
shared libraries, with the triplet in Brightfield's own source,
scripts/duckdb-managed/triplets/arm64-osx-brightfield.cmake
(https://github.com/meridian-online/brightfield).

OFFER OF THE SOURCE

For at least three years from the last day Meridian distributes this version
of Brightfield, Meridian will give anyone who asks a complete, machine-readable
copy of the source code of the GEOS in it, with the changes above, for no more
than the cost of making and sending the copy. Ask at
https://github.com/meridian-online/brightfield/issues.
""",
    )


# --- DuckDB and FineType -------------------------------------------------------


def duckdb(out, duckdb_src):
    """The licence files in DuckDB's source tree whose code the library holds."""
    found = []
    for directory, dirs, files in os.walk(duckdb_src):
        dirs[:] = sorted(d for d in dirs if d not in DUCKDB_SKIP_DIRS)
        for name in sorted(files):
            if LICENCE_FILE.match(name):
                found.append(os.path.relpath(os.path.join(directory, name), duckdb_src))
    if "LICENSE" not in found:
        fail(f"no LICENSE at the top of DuckDB's source, {duckdb_src}")
    for relative in found:
        dest = os.path.join(out, "duckdb", relative)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        shutil.copyfile(os.path.join(duckdb_src, relative), dest)
    return found


# --- Rust crates ---------------------------------------------------------------


def spdx_ids(expression):
    """The licence and exception ids an SPDX expression names. cargo's old
    "MIT/Apache-2.0" form means the same as "MIT OR Apache-2.0"."""
    tokens = re.findall(r"[A-Za-z0-9.+\-]+", expression.replace("/", " OR "))
    return [t.rstrip("+") for t in tokens if t not in ("AND", "OR", "WITH")]


def licence_files(package):
    directory = os.path.dirname(package["manifest_path"])
    paths = []
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        if not LICENCE_FILE.match(name):
            continue
        if os.path.isdir(path):
            for inner in sorted(os.listdir(path)):
                if os.path.isfile(os.path.join(path, inner)):
                    paths.append(os.path.join(path, inner))
        elif os.path.isfile(path):
            paths.append(path)
    if package.get("license_file"):
        path = os.path.normpath(os.path.join(directory, package["license_file"]))
        if os.path.isfile(path) and path not in paths:
            paths.append(path)
    return [(os.path.relpath(p, directory), p) for p in paths]


def crate_source(package, workspace_root, home, revision):
    """Where the source of exactly this version is: its page on crates.io, the
    git repository and commit it was built from, or, for a path dependency
    such as Brightfield's patched libduckdb-sys, the place in its own
    workspace's repository it is vendored at, which may differ from the
    project it came from."""
    source = package.get("source") or ""
    if source.startswith("registry+") or source.startswith("sparse+"):
        return f"https://crates.io/crates/{package['name']}/{package['version']}"
    if source.startswith("git+"):
        url, _, commit = source[len("git+"):].partition("#")
        return f"{url.split('?')[0]} at commit {commit}"
    path = os.path.relpath(os.path.dirname(package["manifest_path"]), workspace_root)
    vendored = f"{path} in {home} at commit {revision}"
    if package.get("repository"):
        vendored += f", vendored, and possibly changed, from {package['repository']}"
    return vendored


def shipped_packages(manifest_path, package_name, features):
    command = [
        "cargo", "metadata", "--format-version", "1", "--locked",
        "--filter-platform", TARGET, "--manifest-path", manifest_path,
    ]
    if features:
        command += ["--features", features]
    metadata = json.loads(subprocess.check_output(command))
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    members = set(metadata["workspace_members"])
    roots = [i for i in members if packages[i]["name"] == package_name]
    if len(roots) != 1:
        fail(f"no one package {package_name} in the workspace of {manifest_path}")
    # The normal dependencies, followed from the package: what is compiled into
    # its build. Build scripts and their dependencies run only at build time,
    # and dev-dependencies only in tests.
    seen, stack = set(), list(roots)
    while stack:
        node = stack.pop()
        if node in seen:
            continue
        seen.add(node)
        for dep in nodes[node]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    # The workspace's own crates are Meridian's, under the licence the bundle
    # carries for them.
    return metadata["workspace_root"], sorted(
        (packages[i] for i in seen if i not in members),
        key=lambda p: (p["name"], p["version"]),
    )


def rust(out_file, manifest_path, package_name, features, title, home):
    workspace_root, crates = shipped_packages(manifest_path, package_name, features)
    revision = subprocess.check_output(["git", "-C", workspace_root, "rev-parse", "HEAD"], text=True).strip()
    known = {}  # a licence file's text, by hash, to the crate that first carried it
    needed = []  # SPDX ids whose text the file's end carries
    lines = [
        title,
        "=" * len(title),
        "",
        f"The {len(crates)} Rust crates compiled into it that Meridian did not write, each with",
        "its licence. A crate whose package carries licence files has them here;",
        "one whose package carries none has its licence named, and the text of",
        "each licence it names is at the end of this file. A licence file the same",
        "as one shown already says whose it is the same as.",
        "",
    ]
    for crate in crates:
        expression = crate.get("license") or ""
        files = licence_files(crate)
        if not expression and not files:
            fail(f"the crate {crate['name']} {crate['version']} declares no licence and carries no licence file")
        lines += [
            "-" * 78,
            f"{crate['name']} {crate['version']}",
            f"Licence: {expression or 'see its licence file'}",
            f"Source: {crate_source(crate, workspace_root, home, revision)}",
        ]
        if not files:
            ids = spdx_ids(expression)
            for spdx in ids:
                if not os.path.isfile(os.path.join(SPDX_DIR, f"{spdx}.txt")):
                    fail(
                        f"{crate['name']} {crate['version']} carries no licence file, and there is no text for "
                        f"{spdx} in {SPDX_DIR}: add it from SPDX's license-list-data"
                    )
                if spdx not in needed:
                    needed.append(spdx)
            lines += [
                "Its package carries no licence file; the text of "
                + ", ".join(ids)
                + " is at the end of this file.",
                "",
            ]
            continue
        for relative, path in files:
            text = read(path).strip()
            digest = hashlib.sha256(re.sub(r"\s+", " ", text).encode()).hexdigest()
            lines.append("")
            if digest in known:
                lines.append(f"{relative}: the same text as {known[digest]}, above.")
                continue
            known[digest] = f"{relative} of {crate['name']} {crate['version']}"
            lines += [f"{relative}:", "", text]
        lines.append("")
    if needed:
        lines += ["=" * 78, "The licences named above whose text a crate's package does not carry", ""]
        for spdx in sorted(needed):
            lines += ["-" * 78, spdx, "", read(os.path.join(SPDX_DIR, f"{spdx}.txt")).strip(), ""]
    write(out_file, "\n".join(lines) + "\n")
    return len(crates)


def deny(manifest_path, config):
    """Hold a package's crates to Brightfield's licence policy, deny.toml's
    allow list, as the gate's licence step holds Brightfield's own."""
    subprocess.run(
        [
            "cargo", "deny", "--manifest-path", manifest_path, "--target", TARGET,
            "--exclude-dev", "--locked", "--config", config, "check", "licenses",
        ],
        check=True,
    )


# --- Commands ------------------------------------------------------------------


def managed(args):
    out = os.path.abspath(args.out)
    if os.path.exists(out):
        shutil.rmtree(out)
    os.makedirs(out)
    installed = os.path.join(args.duckdb_build, "vcpkg_installed")
    triplet_dir = os.path.join(installed, args.triplet)
    ports = installed_ports(installed)
    c_libraries(out, triplet_dir, ports)
    geos(out, args.duckdb_src, triplet_dir, args.lib_dir, ports, args.extensions_cmake, args.vcpkg_commit)
    duckdb(out, args.duckdb_src)

    finetype_licence = os.path.join(args.finetype_src, "LICENSE")
    if not os.path.isfile(finetype_licence):
        fail(f"no LICENSE in FineType's source, {args.finetype_src}")
    write(os.path.join(out, "finetype", "LICENSE"), read(finetype_licence))
    # The extension is compiled into libduckdb.dylib; the command line is the
    # helper arc runs, which FineType's release builds from the same tag.
    for crate, package, file, where in (
        ("finetype-duckdb", "finetype_duckdb", "finetype-in-libduckdb.txt",
         "FineType's DuckDB extension, compiled into libduckdb.dylib"),
        ("finetype-cli", "finetype-cli", "finetype-command-line.txt",
         "FineType's command line, Contents/Helpers/finetype"),
    ):
        manifest = os.path.join(args.finetype_src, "crates", crate, "Cargo.toml")
        deny(manifest, args.deny_config)
        rust(os.path.join(out, "rust-crates", file), manifest, package, None, where,
             "https://github.com/meridian-online/finetype")


def index(args):
    directory = os.path.abspath(args.dir)
    entries = [
        ("GEOS/", "GEOS, under the LGPL version 2.1: its licence, where its source is, and Meridian's offer of a copy (SOURCE.txt)"),
        ("c-libraries/", "the C libraries compiled into libduckdb.dylib, each with the licence file its vcpkg port installs"),
        ("duckdb/", "DuckDB's licence, and those of the code DuckDB and its extensions carry, at their paths in DuckDB's source"),
        ("finetype/", "FineType's licence"),
        ("rust-crates/brightfield.txt", "the Rust crates compiled into Brightfield, Contents/MacOS/brightfield and Contents/Helpers/brightfield-helper"),
        ("rust-crates/finetype-in-libduckdb.txt", "the Rust crates compiled into libduckdb.dylib with FineType's DuckDB extension"),
        ("rust-crates/finetype-command-line.txt", "the Rust crates compiled into FineType's command line, Contents/Helpers/finetype"),
    ]
    missing = [name for name, _ in entries if not os.path.exists(os.path.join(directory, name))]
    if missing:
        fail(f"{directory} has no {', '.join(missing)}")
    lines = [
        "Third-party software in Brightfield",
        "===================================",
        "",
        "Brightfield is Meridian's, under the licence in Contents/Resources/LICENSE.",
        "This app also carries software others wrote, under their own licences,",
        "which are in this directory:",
        "",
    ]
    for name, what in entries:
        lines += [f"  {name}", f"      {what}", ""]
    lines += [
        "GEOS is the one library here under the GNU LGPL. It ships as its own",
        "libraries in Contents/Frameworks, so it can be replaced; GEOS/SOURCE.txt",
        "says where its source is and how to ask Meridian for a copy. A Rust crate",
        "under the Mozilla Public License 2.0 is unchanged from the version its",
        "entry names, and its source is at the address the entry gives.",
    ]
    write(os.path.join(directory, "README.txt"), "\n".join(lines) + "\n")


def main():
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    m = commands.add_parser("managed")
    m.add_argument("out")
    m.add_argument("--duckdb-src", required=True)
    m.add_argument("--duckdb-build", required=True)
    m.add_argument("--triplet", default="arm64-osx-brightfield")
    m.add_argument("--lib-dir", required=True)
    m.add_argument("--extensions-cmake", required=True)
    m.add_argument("--vcpkg-commit", required=True)
    m.add_argument("--finetype-src", required=True)
    m.add_argument("--deny-config", required=True)
    r = commands.add_parser("rust")
    r.add_argument("out_file")
    r.add_argument("--manifest-path", required=True)
    r.add_argument("--package", required=True)
    r.add_argument("--features")
    r.add_argument("--title", required=True)
    r.add_argument("--home", required=True, help="the repository a path dependency is vendored in")
    i = commands.add_parser("index")
    i.add_argument("dir")
    args = parser.parse_args()
    if args.command == "managed":
        managed(args)
    elif args.command == "rust":
        count = rust(args.out_file, args.manifest_path, args.package, args.features, args.title, args.home)
        print(f"notices: {count} crates in {args.out_file}", file=sys.stderr)
    else:
        index(args)


if __name__ == "__main__":
    main()
