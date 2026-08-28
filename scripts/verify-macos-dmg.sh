#!/usr/bin/env bash
# Audit a packaged macOS dmg from any host.
#
# The shipped alpha dmg linked libusb from an absolute Homebrew path, which exists on the CI
# runner but not on a user's Mac, so the app died at load with only a generic "cannot be opened"
# dialog. This script inspects the bundle inside a dmg without needing macOS, so a candidate
# build can be checked before it reaches users.
#
# Usage: scripts/verify-macos-dmg.sh <path-to-dmg>

set -euo pipefail

dmg=${1:?usage: verify-macos-dmg.sh <path-to-dmg>}
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

python3 - "$binary" "$plist" <<'PY' || status=1
import plistlib, re, struct, sys

binary, plist_path = sys.argv[1], sys.argv[2]
blob = open(binary, "rb").read()
failures = []

MH_MAGIC_64, MH_CIGAM_64 = 0xFEEDFACF, 0xCFFAEDFE
FAT_MAGIC, FAT_MAGIC_64 = 0xCAFEBABE, 0xCAFEBABF
CSMAGIC_EMBEDDED_SIGNATURE = 0xFADE0CC0


def slices(data):
    """Yield (label, offset) for each Mach-O in the file.

    A release dmg may ship a universal binary, and every slice in it has its own load
    commands: auditing only the first would pass a build whose other architecture links a
    Homebrew path, which is exactly the failure this script exists to catch.
    """
    if len(data) < 8:
        return
    magic = struct.unpack(">I", data[:4])[0]
    if magic not in (FAT_MAGIC, FAT_MAGIC_64):
        yield ("", 0)
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
        name = {0x0100000C: "arm64", 0x01000007: "x86_64"}.get(cpu & 0xFFFFFFFF, hex(cpu))
        yield (name, off)


def audit(data, base, label):
    """Collect dylibs and the signature blob for one Mach-O slice."""
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
    offset, dylibs, sig = base + 32, [], None
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

    # A CMS blob (slot 0x10000) is what distinguishes a real signature from an ad-hoc seal.
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
    slots = {struct.unpack(">II", sig_blob[12 + i * 8:20 + i * 8])[0] for i in range(count)}
    print(f"   signature{tag}: Developer ID (CMS present)" if 0x10000 in slots
          else f"   signature{tag}: ad-hoc only, Gatekeeper will warn (not fatal)")


found = list(slices(blob))
if not found:
    failures.append("file is too small to be a Mach-O binary")
if len(found) > 1:
    print(f"   universal binary: {len(found)} slices ({', '.join(n for n, _ in found)})")
for label, off in found:
    audit(blob, off, label)

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
