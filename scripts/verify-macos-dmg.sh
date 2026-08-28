#!/usr/bin/env bash
# Audit a packaged macOS dmg from any host.
#
# The shipped alpha dmg linked libusb from an absolute Homebrew path, which exists on the CI
# runner but not on a user's Mac, so the app died at load with only a generic "cannot be opened"
# dialog. This script inspects the bundle inside a dmg without needing macOS, so a candidate
# build can be checked before it reaches users.
#
# Usage: scripts/verify-macos-dmg.sh [--require-developer-id] <path-to-dmg>

set -euo pipefail

require_developer_id=0
dmg=""
while [ "$#" -gt 0 ]; do
	case "$1" in
	--require-developer-id) require_developer_id=1 ;;
	-*) echo "unknown option: $1" >&2; exit 2 ;;
	*) dmg="$1" ;;
	esac
	shift
done

[ -n "$dmg" ] || { echo "usage: verify-macos-dmg.sh [--require-developer-id] <path-to-dmg>" >&2; exit 2; }
[ -f "$dmg" ] || { echo "no such file: $dmg" >&2; exit 2; }

command -v 7z >/dev/null || { echo "7z is required to extract the dmg" >&2; exit 2; }

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

7z x -o"$workdir" "$dmg" >/dev/null 2>&1 || {
	echo "failed to extract $dmg" >&2; exit 2; }

contents=$(find "$workdir" -type d -path '*.app/Contents' | head -1)
[ -n "$contents" ] || { echo "no app bundle found in $dmg" >&2; exit 2; }

plist="$contents/Info.plist"
[ -f "$plist" ] || { echo "app bundle has no Info.plist in $dmg" >&2; exit 2; }

# Audit the executable the bundle actually launches. Picking the first file under MacOS/ would
# silently audit a helper tool instead of the app on any bundle that ships more than one.
main=$(python3 -c 'import plistlib,sys; print(plistlib.load(open(sys.argv[1],"rb")).get("CFBundleExecutable",""))' "$plist")
if [ -n "$main" ] && [ -f "$contents/MacOS/$main" ]; then
	binary="$contents/MacOS/$main"
else
	binary=$(find "$contents/MacOS" -type f | head -1)
fi
[ -n "$binary" ] || { echo "no app bundle executable found in $dmg" >&2; exit 2; }

status=0

echo "== $(basename "$dmg")"
echo "-- bundle: $(basename "$(dirname "$contents")")"

python3 - "$binary" "$plist" "$contents" "$require_developer_id" <<'PY' || status=1
import os, plistlib, posixpath, re, struct, sys

binary, plist_path, contents_dir = sys.argv[1], sys.argv[2], sys.argv[3]
require_developer_id = sys.argv[4] == "1"
blob = open(binary, "rb").read()
failures = []

MH_MAGIC_64, MH_CIGAM_64 = 0xFEEDFACF, 0xCFFAEDFE
FAT_MAGIC, FAT_MAGIC_64 = 0xCAFEBABE, 0xCAFEBABF
CSMAGIC_EMBEDDED_SIGNATURE = 0xFADE0CC0
CSMAGIC_BLOBWRAPPER = 0xFADE0B01
CSSLOT_CMS_SIGNATURE = 0x10000
CPU_NAMES = {0x0100000C: "arm64", 0x01000007: "x86_64"}

# The executable lives in Contents/MacOS, which is what dyld substitutes for both
# @executable_path and (for the main binary) @loader_path.
EXECUTABLE_DIR = os.path.join(contents_dir, "MacOS")


def slices(data):
    """Yield (label, offset, cputype) for each Mach-O in the file.

    A release dmg may ship a universal binary, and every slice in it has its own load
    commands: auditing only the first would pass a build whose other architecture links a
    Homebrew path, which is exactly the failure this script exists to catch.
    """
    if len(data) < 8:
        return
    magic = struct.unpack(">I", data[:4])[0]
    if magic not in (FAT_MAGIC, FAT_MAGIC_64):
        cpu = None
        if len(data) >= 8 and struct.unpack("<I", data[:4])[0] == MH_MAGIC_64:
            cpu = struct.unpack("<I", data[4:8])[0]
        yield ("", 0, cpu)
        return
    # Fat headers are always big-endian; the 64-bit variant widens offset/size to 8 bytes.
    wide = magic == FAT_MAGIC_64
    count = struct.unpack(">I", data[4:8])[0]
    entry = 32 if wide else 20
    for i in range(count):
        base = 8 + i * entry
        if wide:
            cpu, _sub, off = struct.unpack(">iiQ", data[base:base + 16])
        else:
            cpu, _sub, off = struct.unpack(">iiI", data[base:base + 12])
        cpu &= 0xFFFFFFFF
        yield (CPU_NAMES.get(cpu, hex(cpu)), off, cpu)


def slice_cputypes(path):
    """Return the set of cputypes a Mach-O file on disk provides, or None if unreadable."""
    try:
        with open(path, "rb") as fh:
            data = fh.read()
    except OSError:
        return None
    found = {cpu for _label, _off, cpu in slices(data) if cpu is not None}
    return found or None


def resolve(path, rpaths, tag):
    """Resolve a dyld install name to concrete candidate paths inside the bundle.

    dyld substitutes @executable_path and @loader_path directly, and tries every LC_RPATH
    entry in order for @rpath. Treating all of them as "bundle-relative and therefore fine"
    is what let a build ship that referenced a dylib nobody ever copied into the bundle: the
    audit passed and the app still died at launch.
    """
    if path.startswith("@executable_path/"):
        return [os.path.join(EXECUTABLE_DIR, path[len("@executable_path/"):])]
    if path.startswith("@loader_path/"):
        return [os.path.join(EXECUTABLE_DIR, path[len("@loader_path/"):])]
    if path.startswith("@rpath/"):
        suffix = path[len("@rpath/"):]
        out = []
        for rp in rpaths:
            if rp.startswith("@executable_path/"):
                base = os.path.join(EXECUTABLE_DIR, rp[len("@executable_path/"):])
            elif rp == "@executable_path":
                base = EXECUTABLE_DIR
            elif rp.startswith("@loader_path/"):
                base = os.path.join(EXECUTABLE_DIR, rp[len("@loader_path/"):])
            elif rp == "@loader_path":
                base = EXECUTABLE_DIR
            elif rp.startswith("/"):
                # An absolute rpath resolves against the user's filesystem, not the bundle.
                # Only the system locations exist everywhere.
                out.append(("absolute", rp.rstrip("/") + "/" + suffix))
                continue
            else:
                base = os.path.join(EXECUTABLE_DIR, rp)
            out.append(("bundle", os.path.normpath(os.path.join(base, suffix))))
        return out
    return []


def check_bundle_relative(dylibs, rpaths, cputype, tag):
    """Verify every bundle-relative dependency is really present, for this architecture."""
    for path in dylibs:
        if not path.startswith(("@rpath/", "@executable_path/", "@loader_path/")):
            continue
        candidates = resolve(path, rpaths, tag)
        if path.startswith(("@executable_path/", "@loader_path/")):
            candidates = [("bundle", c) for c in candidates]
        if not candidates:
            failures.append(
                f"{path}{tag} cannot be resolved: the binary declares no LC_RPATH entry")
            continue

        system_fallback = [c for kind, c in candidates
                           if kind == "absolute" and c.startswith(("/System/", "/usr/lib/"))]
        present = [c for kind, c in candidates if kind == "bundle" and os.path.isfile(c)]
        if not present:
            if system_fallback:
                continue
            searched = ", ".join(
                os.path.relpath(c, contents_dir) if kind == "bundle" else c
                for kind, c in candidates)
            failures.append(
                f"{path}{tag} is not in the bundle; dyld would search: {searched}")
            continue

        if cputype is None:
            continue
        target = present[0]
        provided = slice_cputypes(target)
        rel = os.path.relpath(target, contents_dir)
        if provided is None:
            failures.append(f"{rel}{tag} is not a readable Mach-O library")
        elif cputype not in provided:
            have = ", ".join(sorted(CPU_NAMES.get(c, hex(c)) for c in provided))
            failures.append(
                f"{rel}{tag} has no {CPU_NAMES.get(cputype, hex(cputype))} slice (provides: {have})")


def check_signature(data, base, sig, tag):
    """Classify the code signature, verifying a claimed CMS blob really exists.

    The signature index is only a table of contents. An entry for slot 0x10000 with nothing
    behind it is not a Developer ID signature, and reporting it as one told the operator that
    an unsigned build was signed.
    """
    if sig is None:
        failures.append(f"binary{tag} is not code signed at all")
        return
    # LC_CODE_SIGNATURE stores an offset relative to the start of its own slice, not to the start
    # of the file. In a thin binary `base` is 0 and the two coincide, which is why this only shows
    # up on universal binaries: reading at the absolute offset lands in unrelated bytes and the
    # signature looks truncated or absent.
    sig_start = base + sig[0]
    sig_blob = data[sig_start:sig_start + sig[1]]
    if len(sig_blob) < 12:
        failures.append(f"code signature{tag} is truncated")
        return
    if struct.unpack(">I", sig_blob[0:4])[0] != CSMAGIC_EMBEDDED_SIGNATURE:
        failures.append(f"code signature{tag} is not an embedded signature superblob")
        return
    count = struct.unpack(">I", sig_blob[8:12])[0]
    if len(sig_blob) < 12 + count * 8:
        failures.append(f"code signature index{tag} is truncated")
        return
    slots = {}
    for i in range(count):
        slot, off = struct.unpack(">II", sig_blob[12 + i * 8:20 + i * 8])
        slots[slot] = off

    if CSSLOT_CMS_SIGNATURE not in slots:
        if require_developer_id:
            failures.append(
                f"signature{tag} is ad-hoc; a release build must carry a Developer ID signature")
        else:
            print(f"   signature{tag}: ad-hoc only, Gatekeeper will warn (not fatal)")
        return

    off = slots[CSSLOT_CMS_SIGNATURE]
    if off + 8 > len(sig_blob):
        failures.append(
            f"signature{tag} indexes a CMS blob at offset {off}, past the end of the signature")
        return
    magic, length = struct.unpack(">II", sig_blob[off:off + 8])
    if magic != CSMAGIC_BLOBWRAPPER:
        failures.append(
            f"signature{tag} claims a CMS slot but the blob magic is {magic:#x}, "
            f"not a CMS blob wrapper")
        return
    if length < 8 or off + length > len(sig_blob):
        failures.append(f"signature{tag} has a CMS blob whose length {length} is out of range")
        return
    payload = sig_blob[off + 8:off + length]
    if not payload:
        # An empty wrapper is what `codesign --sign -` leaves behind on some toolchains: the
        # slot exists, the certificate does not.
        failures.append(f"signature{tag} has an empty CMS blob; nothing is actually signed")
        return
    # A CMS SignedData is DER: a constructed SEQUENCE. Anything else is not a signature.
    if payload[0] != 0x30:
        failures.append(
            f"signature{tag} CMS blob is not DER-encoded (leading byte {payload[0]:#04x})")
        return
    print(f"   signature{tag}: Developer ID (CMS blob, {len(payload)} bytes)")


def audit(data, base, label, cputype):
    """Collect dylibs, rpaths and the signature blob for one Mach-O slice."""
    tag = f" [{label}]" if label else ""
    if len(data) < base + 32:
        failures.append(f"truncated Mach-O{tag}")
        return
    magic = struct.unpack("<I", data[base:base + 4])[0]
    if magic != MH_MAGIC_64:
        # 32-bit or byte-swapped: not something this project ships, and not something this
        # parser can read. Report it rather than passing a binary that was never audited.
        kind = "big-endian" if magic == MH_CIGAM_64 else f"unrecognised (magic {magic:#x})"
        failures.append(f"binary{tag} is not a little-endian 64-bit Mach-O: {kind}")
        return

    ncmds = struct.unpack("<I", data[base + 16:base + 20])[0]
    offset, dylibs, rpaths, sig = base + 32, [], [], None
    for _ in range(ncmds):
        if offset + 8 > len(data):
            failures.append(f"load commands{tag} run past the end of the file")
            return
        cmd, size = struct.unpack("<II", data[offset:offset + 8])
        if size < 8:
            failures.append(f"malformed load command{tag} of size {size}")
            return
        if cmd in (0x0C, 0x18, 0x8000001F):  # LOAD_DYLIB, LAZY_LOAD_DYLIB, LOAD_UPWARD_DYLIB
            name_off = struct.unpack("<I", data[offset + 8:offset + 12])[0]
            raw = data[offset + name_off:offset + size].split(b"\0")[0]
            dylibs.append(raw.decode("utf-8", "replace"))
        elif cmd == 0x8000001C:  # LC_RPATH
            name_off = struct.unpack("<I", data[offset + 8:offset + 12])[0]
            raw = data[offset + name_off:offset + size].split(b"\0")[0]
            rpaths.append(raw.decode("utf-8", "replace"))
        elif cmd == 0x1D:  # LC_CODE_SIGNATURE
            sig = struct.unpack("<II", data[offset + 8:offset + 16])
        offset += size

    # Anything outside these prefixes is absent on a clean Mac and aborts the app before main.
    external = [d for d in dylibs
                if not d.startswith(("/System/", "/usr/lib/", "@rpath/", "@executable_path/",
                                     "@loader_path/"))]
    if external:
        failures.append(f"links libraries{tag} that a clean Mac will not have:\n     "
                        + "\n     ".join(external))
    else:
        print(f"   dylibs{tag}: {len(dylibs)} linked, all system or bundle-relative")

    before = len(failures)
    check_bundle_relative(dylibs, rpaths, cputype, tag)
    if len(failures) == before:
        relative = [d for d in dylibs
                    if d.startswith(("@rpath/", "@executable_path/", "@loader_path/"))]
        if relative:
            print(f"   bundled{tag}: {len(relative)} bundle-relative dependencies resolved")

    check_signature(data, base, sig, tag)


found = list(slices(blob))
if not found:
    failures.append("file is too small to be a Mach-O binary")
if len(found) > 1:
    print(f"   universal binary: {len(found)} slices ({', '.join(n for n, _, _ in found)})")
for label, off, cpu in found:
    audit(blob, off, label, cpu)

info = plistlib.load(open(plist_path, "rb"))
version = info.get("CFBundleShortVersionString", "")
build = info.get("CFBundleVersion", "")
# Launch Services rejects bundles whose version is not a dotted numeric string.
for key, value in (("CFBundleShortVersionString", version), ("CFBundleVersion", build)):
    if not re.fullmatch(r"\d+(\.\d+)*", value or ""):
        failures.append(f"{key} is not a valid version: {value!r}")
if not failures:
    print(f"   version: {version} (build {build})")

for failure in failures:
    print(f"   FAIL: {failure}")
sys.exit(1 if failures else 0)
PY

if [ "$status" -eq 0 ]; then
	echo "   OK: bundle is self-contained and its metadata is valid"
else
	echo "   dmg would fail on a clean Mac" >&2
fi
exit "$status"
