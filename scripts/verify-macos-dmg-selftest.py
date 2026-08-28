#!/usr/bin/env python3
"""Build synthetic macOS app bundles and run verify-macos-dmg.sh against them.

The script exists to catch a dmg that would die on a user's Mac. It has never been run
against a bundle that is actually broken in the ways it claims to detect, so this harness
constructs each case as a real 64-bit Mach-O with hand-built load commands.
"""
import os
import plistlib
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(ROOT, "scripts", "verify-macos-dmg.sh")

LC_LOAD_DYLIB = 0x0C
LC_CODE_SIGNATURE = 0x1D
MH_MAGIC_64 = 0xFEEDFACF


def dylib_cmd(name: str) -> bytes:
    """A real LC_LOAD_DYLIB: header, 24-byte fixed part, then the NUL-padded path."""
    raw = name.encode() + b"\0"
    pad = (-len(raw)) % 8
    body = struct.pack("<IIII", 24, 0, 0, 0) + raw + b"\0" * pad
    size = 8 + len(body)
    return struct.pack("<II", LC_LOAD_DYLIB, size) + body


def codesig_cmd(offset: int, size: int) -> bytes:
    return struct.pack("<IIII", LC_CODE_SIGNATURE, 16, offset, size)


def superblob(slots) -> bytes:
    """A SuperBlob whose index lists the given slot types. 0x10000 marks a CMS signature."""
    count = len(slots)
    out = struct.pack(">III", 0xFADE0CC0, 0, count)
    for slot in slots:
        out += struct.pack(">II", slot, 0)
    return out


def build_macho(dylibs, sig_slots=None, signed=True) -> bytes:
    cmds = b"".join(dylib_cmd(d) for d in dylibs)
    ncmds = len(dylibs)

    if signed:
        # The signature blob sits past the load commands; reserve its offset first.
        sig_cmd_size = 16
        header_len = 32 + len(cmds) + sig_cmd_size
        blob = superblob(sig_slots if sig_slots is not None else [0x10000])
        cmds += codesig_cmd(header_len, len(blob))
        ncmds += 1
    else:
        header_len = 32 + len(cmds)
        blob = b""

    # magic, cputype (ARM64), cpusubtype (ALL|PTR_AUTH), filetype MH_EXECUTE, ncmds,
    # sizeofcmds, flags, reserved. Packed unsigned: the subtype's high bit is a flag.
    header = struct.pack(
        "<IIIIIIII", MH_MAGIC_64, 0x0100000C, 0x00000000, 2, ncmds, len(cmds), 0x00200085, 0
    )
    return header + cmds + blob


def fat(slices_, wide=False):
    """Wrap thin Mach-O slices in a universal binary header.

    Fat headers are big-endian regardless of the slices' byte order, and each slice is page
    aligned. FAT_MAGIC_64 widens the offset and size fields to 8 bytes.
    """
    magic = 0xCAFEBABF if wide else 0xCAFEBABE
    entry = 32 if wide else 20
    head_len = 8 + entry * len(slices_)
    align = 4096
    offsets, cur = [], (head_len + align - 1) // align * align
    for body in slices_:
        offsets.append(cur)
        cur = (cur + len(body) + align - 1) // align * align

    out = struct.pack(">II", magic, len(slices_))
    cpus = [0x0100000C, 0x01000007]
    for i, body in enumerate(slices_):
        cpu = cpus[i % len(cpus)]
        if wide:
            out += struct.pack(">iiQQQ", cpu, 0, offsets[i], len(body), 12)
        else:
            out += struct.pack(">iiIII", cpu, 0, offsets[i], len(body), 12)

    blob = bytearray(out)
    for off, body in zip(offsets, slices_):
        if len(blob) < off:
            blob.extend(b"\0" * (off - len(blob)))
        blob[off:off + len(body)] = body
    return bytes(blob)


def make_dmg(tmp: str, name: str, binary: bytes, info: dict) -> str:
    """7z reads a plain zip, and the script only ever asks 7z to extract, so a zip stands in
    for a dmg without needing macOS tooling to author a real one."""
    stage = os.path.join(tmp, f"stage-{name}")
    macos = os.path.join(stage, "GemImager.app", "Contents", "MacOS")
    os.makedirs(macos, exist_ok=True)

    exe = os.path.join(macos, "gem-imager-gui")
    with open(exe, "wb") as fh:
        fh.write(binary)
    os.chmod(exe, 0o755)

    with open(os.path.join(stage, "GemImager.app", "Contents", "Info.plist"), "wb") as fh:
        plistlib.dump(info, fh)

    dmg = os.path.join(tmp, f"{name}.dmg")
    subprocess.run(
        ["7z", "a", "-tzip", dmg, "."], cwd=stage, check=True, capture_output=True
    )
    return dmg


def plist(version="0.9.0", build="0.9.0") -> dict:
    return {
        "CFBundleShortVersionString": version,
        "CFBundleVersion": build,
        "CFBundleExecutable": "gem-imager-gui",
        "CFBundleIdentifier": "org.beagleboard.gem-imager",
    }


SYSTEM_LIBS = [
    "/usr/lib/libSystem.B.dylib",
    "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation",
    "@rpath/libiced.dylib",
]

CASES = [
    dict(
        name="clean",
        why="a self-contained, signed, correctly versioned bundle must pass",
        binary=lambda: build_macho(SYSTEM_LIBS),
        info=plist(),
        exit=0,
        must=["OK: bundle is self-contained"],
        must_not=["FAIL"],
    ),
    dict(
        name="homebrew-libusb",
        why="the actual alpha failure: an absolute Homebrew path is absent on a user's Mac",
        binary=lambda: build_macho(SYSTEM_LIBS + ["/opt/homebrew/opt/libusb/lib/libusb-1.0.0.dylib"]),
        info=plist(),
        exit=1,
        must=["FAIL", "libusb-1.0.0.dylib", "clean Mac"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="usr-local-lib",
        why="the Intel Homebrew prefix must be caught too, not just /opt/homebrew",
        binary=lambda: build_macho(SYSTEM_LIBS + ["/usr/local/lib/libusb-1.0.0.dylib"]),
        info=plist(),
        exit=1,
        must=["FAIL", "/usr/local/lib/libusb-1.0.0.dylib"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="unsigned",
        why="a bundle with no LC_CODE_SIGNATURE is not shippable",
        binary=lambda: build_macho(SYSTEM_LIBS, signed=False),
        info=plist(),
        exit=1,
        must=["FAIL", "not code signed"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="adhoc-signed",
        why="an ad-hoc seal warns in Gatekeeper but still launches, so it must not fail",
        binary=lambda: build_macho(SYSTEM_LIBS, sig_slots=[0, 2]),
        info=plist(),
        exit=0,
        must=["ad-hoc only"],
        must_not=["FAIL"],
    ),
    dict(
        name="bad-version",
        why="Launch Services rejects a non-numeric CFBundleShortVersionString",
        binary=lambda: build_macho(SYSTEM_LIBS),
        info=plist(version="0.9.0-alpha"),
        exit=1,
        must=["FAIL", "CFBundleShortVersionString"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="missing-version",
        why="an absent version key must be reported, not silently accepted",
        binary=lambda: build_macho(SYSTEM_LIBS),
        info={"CFBundleExecutable": "gem-imager-gui"},
        exit=1,
        must=["FAIL"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="universal-clean",
        why="a universal binary with two clean slices must pass and be reported as such",
        binary=lambda: fat([build_macho(SYSTEM_LIBS), build_macho(SYSTEM_LIBS)]),
        info=plist(),
        exit=0,
        must=["universal binary: 2 slices", "arm64", "x86_64", "OK: bundle is self-contained"],
        must_not=["FAIL"],
    ),
    dict(
        name="universal-second-slice-dirty",
        why="a Homebrew path in only the second slice must still fail the audit",
        binary=lambda: fat([
            build_macho(SYSTEM_LIBS),
            build_macho(SYSTEM_LIBS + ["/opt/homebrew/opt/libusb/lib/libusb-1.0.0.dylib"]),
        ]),
        info=plist(),
        exit=1,
        must=["FAIL", "libusb-1.0.0.dylib", "x86_64"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="universal-unsigned-slice",
        why="an unsigned slice must be caught even when the other one is signed",
        binary=lambda: fat([
            build_macho(SYSTEM_LIBS),
            build_macho(SYSTEM_LIBS, signed=False),
        ]),
        info=plist(),
        exit=1,
        must=["FAIL", "not code signed"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="universal-fat64",
        why="the FAT_MAGIC_64 header variant must parse like the 32-bit one",
        binary=lambda: fat([build_macho(SYSTEM_LIBS), build_macho(SYSTEM_LIBS)], wide=True),
        info=plist(),
        exit=0,
        must=["universal binary: 2 slices"],
        must_not=["FAIL"],
    ),
    dict(
        name="universal-signature-offset-is-slice-relative",
        why=(
            "LC_CODE_SIGNATURE offsets are relative to their own slice. Reading them as absolute "
            "file offsets lands in unrelated bytes, so a correctly signed universal binary gets "
            "reported as truncated or unsigned. Assert the signature is actually parsed, not just "
            "that the bundle passes: the CMS slot can only be seen if the blob was read correctly."
        ),
        binary=lambda: fat([
            build_macho(SYSTEM_LIBS, sig_slots=[0, 2]),
            build_macho(SYSTEM_LIBS, sig_slots=[0x10000]),
        ]),
        info=plist(),
        exit=0,
        must=["universal binary: 2 slices", "ad-hoc only", "Developer ID (CMS present)"],
        must_not=["FAIL", "truncated", "not code signed"],
    ),
    dict(
        name="not-macho",
        why="an unreadable binary must fail rather than be reported self-contained",
        binary=lambda: b"#!/bin/sh\necho hi\n" + b"\0" * 64,
        info=plist(),
        exit=1,
        must=["FAIL", "not a little-endian 64-bit Mach-O"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="truncated-macho",
        why="a header claiming more load commands than the file holds must not pass",
        binary=lambda: build_macho(SYSTEM_LIBS)[:48],
        info=plist(),
        exit=1,
        must=["FAIL"],
        must_not=["OK: bundle is self-contained"],
    ),
]


def main() -> int:
    if not os.access(SCRIPT, os.X_OK):
        print(f"not executable: {SCRIPT}", file=sys.stderr)
        return 2
    if shutil.which("7z") is None:
        print("7z is required", file=sys.stderr)
        return 2

    failed = 0
    with tempfile.TemporaryDirectory() as tmp:
        for case in CASES:
            dmg = make_dmg(tmp, case["name"], case["binary"](), case["info"])
            proc = subprocess.run(
                [SCRIPT, dmg], capture_output=True, text=True
            )
            out = proc.stdout + proc.stderr
            why = []

            if proc.returncode != case["exit"]:
                why.append(f"exit {proc.returncode}, expected {case['exit']}")
            for needle in case["must"]:
                if needle not in out:
                    why.append(f"missing {needle!r}")
            for needle in case["must_not"]:
                if needle in out:
                    why.append(f"unexpected {needle!r}")

            if why:
                failed += 1
                print(f"  FAIL  {case['name']} - {case['why']}")
                for w in why:
                    print(f"        {w}")
                for line in out.strip().splitlines():
                    print(f"        | {line}")
            else:
                print(f"  PASS  {case['name']} - {case['why']}")

    print()
    if failed:
        print(f"{failed} of {len(CASES)} scenarios failed")
        return 1
    print(f"all {len(CASES)} scenarios passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
