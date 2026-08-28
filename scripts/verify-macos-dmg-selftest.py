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
LC_RPATH = 0x8000001C
MH_MAGIC_64 = 0xFEEDFACF
CSMAGIC_BLOBWRAPPER = 0xFADE0B01
CSSLOT_CMS_SIGNATURE = 0x10000
CPU_ARM64 = 0x0100000C
CPU_X86_64 = 0x01000007


def dylib_cmd(name: str) -> bytes:
    """A real LC_LOAD_DYLIB: header, 24-byte fixed part, then the NUL-padded path."""
    raw = name.encode() + b"\0"
    pad = (-len(raw)) % 8
    body = struct.pack("<IIII", 24, 0, 0, 0) + raw + b"\0" * pad
    size = 8 + len(body)
    return struct.pack("<II", LC_LOAD_DYLIB, size) + body


def codesig_cmd(offset: int, size: int) -> bytes:
    return struct.pack("<IIII", LC_CODE_SIGNATURE, 16, offset, size)


def rpath_cmd(path: str) -> bytes:
    """A real LC_RPATH: header, 4-byte offset, then the NUL-padded path."""
    raw = path.encode() + b"\0"
    pad = (-(len(raw) + 12)) % 8
    body = struct.pack("<I", 12) + raw + b"\0" * pad
    return struct.pack("<II", LC_RPATH, 8 + len(body)) + body


# A minimal DER SEQUENCE. Real CMS SignedData is far larger, but the audit only needs to
# distinguish "a DER structure is present" from "the slot points at nothing".
DER_CMS = bytes([0x30, 0x06, 0x06, 0x04, 0x2A, 0x86, 0x48, 0x86])


def superblob(slots, cms=DER_CMS) -> bytes:
    """A SuperBlob whose index lists the given slot types.

    Slot 0x10000 is only a real CMS signature when the index offset points at a blob wrapper
    holding DER data, so the payload is emitted here rather than leaving a dangling offset.
    Pass ``cms=None`` to build the dangling-index case the audit must now reject, or
    ``cms=b""`` for an empty wrapper.
    """
    count = len(slots)
    header_len = 12 + count * 8
    index = b""
    payload = b""
    for slot in slots:
        if slot == CSSLOT_CMS_SIGNATURE and cms is not None:
            off = header_len + len(payload)
            index += struct.pack(">II", slot, off)
            payload += struct.pack(">II", CSMAGIC_BLOBWRAPPER, 8 + len(cms)) + cms
        else:
            index += struct.pack(">II", slot, 0)
    total = header_len + len(payload)
    return struct.pack(">III", 0xFADE0CC0, total, count) + index + payload


def build_macho(dylibs, sig_slots=None, signed=True, rpaths=(), cms=DER_CMS,
                cpu=CPU_ARM64) -> bytes:
    cmds = b"".join(dylib_cmd(d) for d in dylibs)
    cmds += b"".join(rpath_cmd(r) for r in rpaths)
    ncmds = len(dylibs) + len(rpaths)

    if signed:
        # The signature blob sits past the load commands; reserve its offset first.
        sig_cmd_size = 16
        header_len = 32 + len(cmds) + sig_cmd_size
        blob = superblob(sig_slots if sig_slots is not None else [CSSLOT_CMS_SIGNATURE], cms=cms)
        cmds += codesig_cmd(header_len, len(blob))
        ncmds += 1
    else:
        header_len = 32 + len(cmds)
        blob = b""

    # magic, cputype (ARM64), cpusubtype (ALL|PTR_AUTH), filetype MH_EXECUTE, ncmds,
    # sizeofcmds, flags, reserved. Packed unsigned: the subtype's high bit is a flag.
    header = struct.pack(
        "<IIIIIIII", MH_MAGIC_64, cpu, 0x00000000, 2, ncmds, len(cmds), 0x00200085, 0
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


def make_dmg(tmp: str, name: str, binary: bytes, info: dict, bundled=None) -> str:
    """7z reads a plain zip, and the script only ever asks 7z to extract, so a zip stands in
    for a dmg without needing macOS tooling to author a real one."""
    stage = os.path.join(tmp, f"stage-{name}")
    macos = os.path.join(stage, "GemImager.app", "Contents", "MacOS")
    os.makedirs(macos, exist_ok=True)

    exe = os.path.join(macos, "gem-imager-gui")
    with open(exe, "wb") as fh:
        fh.write(binary)
    os.chmod(exe, 0o755)

    # Libraries the bundle actually ships, keyed by their path under Contents/.
    for rel, payload in (bundled or {}).items():
        dest = os.path.join(stage, "GemImager.app", "Contents", rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "wb") as fh:
            fh.write(payload)

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

# The rpath the release bundles use: dyld resolves @rpath against Contents/Frameworks.
FRAMEWORKS_RPATH = ["@executable_path/../Frameworks"]


def bundled_lib(cpu=CPU_ARM64, cpus=None) -> bytes:
    """A stand-in dylib that really is a Mach-O, so slice checks have something to read."""
    if cpus:
        return fat([build_macho([], cpu=c) for c in cpus])
    return build_macho([], cpu=cpu)


FRAMEWORKS = {"Frameworks/libiced.dylib": bundled_lib()}
FRAMEWORKS_UNIVERSAL = {
    "Frameworks/libiced.dylib": bundled_lib(cpus=[CPU_ARM64, CPU_X86_64])
}


def clean_macho(extra=(), **kw):
    kw.setdefault("rpaths", FRAMEWORKS_RPATH)
    return build_macho(SYSTEM_LIBS + list(extra), **kw)

CASES = [
    dict(
        name="clean",
        why="a self-contained, signed, correctly versioned bundle must pass",
        binary=lambda: clean_macho(),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=0,
        must=["OK: bundle is self-contained", "bundle-relative dependencies resolved"],
        must_not=["FAIL"],
    ),
    dict(
        name="rpath-target-missing",
        why=(
            "the failure the old audit waved through: an @rpath dependency that was never "
            "copied into the bundle. dyld aborts the app at launch, so the audit must fail"
        ),
        binary=lambda: clean_macho(),
        bundled={},
        info=plist(),
        exit=1,
        must=["FAIL", "@rpath/libiced.dylib", "not in the bundle", "Frameworks/libiced.dylib"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="rpath-without-any-lc-rpath",
        why="an @rpath install name with no LC_RPATH entry can never resolve",
        binary=lambda: build_macho(SYSTEM_LIBS, rpaths=[]),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=1,
        must=["FAIL", "declares no LC_RPATH"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="rpath-slice-mismatch",
        why=(
            "a bundled dylib that only ships arm64 cannot satisfy an x86_64 executable; "
            "the app installs fine and dies on Intel Macs"
        ),
        binary=lambda: clean_macho(cpu=CPU_X86_64),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=1,
        must=["FAIL", "no x86_64 slice", "arm64"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="rpath-universal-lib-satisfies-both",
        why="a universal bundled dylib must satisfy both slices of a universal executable",
        binary=lambda: fat([
            clean_macho(cpu=CPU_ARM64),
            clean_macho(cpu=CPU_X86_64),
        ]),
        bundled=FRAMEWORKS_UNIVERSAL,
        info=plist(),
        exit=0,
        must=["universal binary: 2 slices", "OK: bundle is self-contained"],
        must_not=["FAIL"],
    ),
    dict(
        name="executable-path-target-missing",
        why="@executable_path is resolved the same way and must be checked too",
        binary=lambda: build_macho(
            ["/usr/lib/libSystem.B.dylib", "@executable_path/../Frameworks/libmissing.dylib"],
            rpaths=FRAMEWORKS_RPATH,
        ),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=1,
        must=["FAIL", "libmissing.dylib", "not in the bundle"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="rpath-system-absolute-fallback",
        why=(
            "an absolute rpath into /usr/lib resolves on every Mac, so it must not be "
            "reported as a missing bundle dependency"
        ),
        binary=lambda: build_macho(
            ["/usr/lib/libSystem.B.dylib", "@rpath/libobjc.A.dylib"],
            rpaths=["/usr/lib"],
        ),
        bundled={},
        info=plist(),
        exit=0,
        must=["OK: bundle is self-contained"],
        must_not=["FAIL"],
    ),
    dict(
        name="homebrew-libusb",
        why="the actual alpha failure: an absolute Homebrew path is absent on a user's Mac",
        binary=lambda: clean_macho(["/opt/homebrew/opt/libusb/lib/libusb-1.0.0.dylib"]),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=1,
        must=["FAIL", "libusb-1.0.0.dylib", "clean Mac"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="usr-local-lib",
        why="the Intel Homebrew prefix must be caught too, not just /opt/homebrew",
        binary=lambda: clean_macho(["/usr/local/lib/libusb-1.0.0.dylib"]),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=1,
        must=["FAIL", "/usr/local/lib/libusb-1.0.0.dylib"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="unsigned",
        why="a bundle with no LC_CODE_SIGNATURE is not shippable",
        binary=lambda: clean_macho(signed=False),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=1,
        must=["FAIL", "not code signed"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="adhoc-signed",
        why="an ad-hoc seal warns in Gatekeeper but still launches, so it must not fail",
        binary=lambda: clean_macho(sig_slots=[0, 2]),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=0,
        must=["ad-hoc only"],
        must_not=["FAIL"],
    ),
    dict(
        name="adhoc-signed-release-gate",
        why=(
            "a release build must not ship an ad-hoc seal: with --require-developer-id the "
            "same bundle that merely warns above has to fail"
        ),
        binary=lambda: clean_macho(sig_slots=[0, 2]),
        bundled=FRAMEWORKS,
        info=plist(),
        args=["--require-developer-id"],
        exit=1,
        must=["FAIL", "ad-hoc", "Developer ID"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="developer-id-release-gate",
        why="a real CMS signature must satisfy --require-developer-id",
        binary=lambda: clean_macho(),
        bundled=FRAMEWORKS,
        info=plist(),
        args=["--require-developer-id"],
        exit=0,
        must=["Developer ID (CMS blob", "OK: bundle is self-contained"],
        must_not=["FAIL"],
    ),
    dict(
        name="cms-slot-without-blob",
        why=(
            "the old audit called any 0x10000 index entry a Developer ID signature. A slot "
            "whose offset points at nothing is not a signature and must be rejected"
        ),
        binary=lambda: clean_macho(cms=None),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=1,
        must=["FAIL", "CMS"],
        must_not=["Developer ID (CMS blob", "OK: bundle is self-contained"],
    ),
    dict(
        name="cms-empty-blob",
        why=(
            "`codesign --sign -` writes the CMS slot with an empty wrapper, which is what "
            "every unsigned CI build ships. It carries no certificate, so it must not read "
            "as Developer ID, but it launches, so a pre-release must not fail on it"
        ),
        binary=lambda: clean_macho(cms=b""),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=0,
        must=["ad-hoc only", "OK: bundle is self-contained"],
        must_not=["FAIL", "Developer ID (CMS blob"],
    ),
    dict(
        name="cms-empty-blob-release-gate",
        why=(
            "the same empty wrapper is not good enough for a tagged release: "
            "--require-developer-id must reject it"
        ),
        binary=lambda: clean_macho(cms=b""),
        bundled=FRAMEWORKS,
        info=plist(),
        args=["--require-developer-id"],
        exit=1,
        must=["FAIL", "ad-hoc", "CMS blob is empty", "Developer ID"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="cms-not-der",
        why="a CMS blob that is not DER-encoded is not a parseable signature",
        binary=lambda: clean_macho(cms=b"not-der-at-all"),
        bundled=FRAMEWORKS,
        info=plist(),
        exit=1,
        must=["FAIL", "not DER-encoded"],
        must_not=["Developer ID (CMS blob", "OK: bundle is self-contained"],
    ),
    dict(
        name="bad-version",
        why="Launch Services rejects a non-numeric CFBundleShortVersionString",
        binary=lambda: clean_macho(),
        bundled=FRAMEWORKS,
        info=plist(version="0.9.0-alpha"),
        exit=1,
        must=["FAIL", "CFBundleShortVersionString"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="missing-version",
        why="an absent version key must be reported, not silently accepted",
        binary=lambda: clean_macho(),
        bundled=FRAMEWORKS,
        info={"CFBundleExecutable": "gem-imager-gui"},
        exit=1,
        must=["FAIL"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="universal-clean",
        why="a universal binary with two clean slices must pass and be reported as such",
        binary=lambda: fat([
            clean_macho(cpu=CPU_ARM64),
            clean_macho(cpu=CPU_X86_64),
        ]),
        bundled=FRAMEWORKS_UNIVERSAL,
        info=plist(),
        exit=0,
        must=["universal binary: 2 slices", "arm64", "x86_64", "OK: bundle is self-contained"],
        must_not=["FAIL"],
    ),
    dict(
        name="universal-second-slice-dirty",
        why="a Homebrew path in only the second slice must still fail the audit",
        binary=lambda: fat([
            clean_macho(cpu=CPU_ARM64),
            clean_macho(["/opt/homebrew/opt/libusb/lib/libusb-1.0.0.dylib"], cpu=CPU_X86_64),
        ]),
        bundled=FRAMEWORKS_UNIVERSAL,
        info=plist(),
        exit=1,
        must=["FAIL", "libusb-1.0.0.dylib", "x86_64"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="universal-unsigned-slice",
        why="an unsigned slice must be caught even when the other one is signed",
        binary=lambda: fat([
            clean_macho(cpu=CPU_ARM64),
            clean_macho(signed=False, cpu=CPU_X86_64),
        ]),
        bundled=FRAMEWORKS_UNIVERSAL,
        info=plist(),
        exit=1,
        must=["FAIL", "not code signed"],
        must_not=["OK: bundle is self-contained"],
    ),
    dict(
        name="universal-fat64",
        why="the FAT_MAGIC_64 header variant must parse like the 32-bit one",
        binary=lambda: fat([
            clean_macho(cpu=CPU_ARM64),
            clean_macho(cpu=CPU_X86_64),
        ], wide=True),
        bundled=FRAMEWORKS_UNIVERSAL,
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
            "that the bundle passes: the CMS blob can only be read if the offset was resolved "
            "against the right slice."
        ),
        binary=lambda: fat([
            clean_macho(sig_slots=[0, 2], cpu=CPU_ARM64),
            clean_macho(sig_slots=[CSSLOT_CMS_SIGNATURE], cpu=CPU_X86_64),
        ]),
        bundled=FRAMEWORKS_UNIVERSAL,
        info=plist(),
        exit=0,
        must=["universal binary: 2 slices", "ad-hoc only", "Developer ID (CMS blob"],
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
        binary=lambda: clean_macho()[:48],
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
            dmg = make_dmg(tmp, case["name"], case["binary"](), case["info"],
                           case.get("bundled"))
            proc = subprocess.run(
                [SCRIPT, *case.get("args", []), dmg], capture_output=True, text=True
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
