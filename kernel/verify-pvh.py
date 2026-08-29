#!/usr/bin/env python3
"""Verify that an uncompressed Linux vmlinux satisfies the SOMA PVH machine contract.

Checks, all of which fail closed:

- ELF64, little-endian, ET_EXEC, EM_X86_64.
- Every PT_LOAD segment has p_paddr >= MIN_PADDR (0x01000000), ends below 4 GiB,
  and no two PT_LOAD segments overlap in physical address space.
- A PT_NOTE segment contains a note with name "Xen" and type 18
  (XEN_ELFNOTE_PHYS32_ENTRY) whose descriptor is a 4- or 8-byte little-endian
  address that fits in 32 bits.
- That entry address lies inside an executable (PF_X) PT_LOAD segment.

Usage:
    verify-pvh.py VMLINUX [--json]

Only the Python standard library is used.
"""

import argparse
import json
import struct
import sys

MIN_PADDR = 0x01000000
FOUR_GIB = 1 << 32
PT_LOAD = 1
PT_NOTE = 4
PF_X = 1
ET_EXEC = 2
EM_X86_64 = 62
XEN_ELFNOTE_PHYS32_ENTRY = 18


class VerifyError(Exception):
    pass


def read_exact(handle, offset, size, what):
    handle.seek(offset)
    data = handle.read(size)
    if len(data) != size:
        raise VerifyError(f"short read while reading {what}")
    return data


def parse_header(handle):
    ident = read_exact(handle, 0, 16, "ELF identification")
    if ident[:4] != b"\x7fELF":
        raise VerifyError("not an ELF file")
    if ident[4] != 2:
        raise VerifyError("not ELF64")
    if ident[5] != 1:
        raise VerifyError("not little-endian")
    fields = struct.unpack("<HHIQQQIHHHHHH", read_exact(handle, 16, 48, "ELF header"))
    header = {
        "e_type": fields[0],
        "e_machine": fields[1],
        "e_entry": fields[3],
        "e_phoff": fields[4],
        "e_phentsize": fields[8],
        "e_phnum": fields[9],
    }
    if header["e_type"] != ET_EXEC:
        raise VerifyError(f"e_type is {header['e_type']}, expected ET_EXEC")
    if header["e_machine"] != EM_X86_64:
        raise VerifyError(f"e_machine is {header['e_machine']}, expected EM_X86_64")
    if header["e_phentsize"] != 56:
        raise VerifyError("unexpected program header entry size")
    if header["e_phnum"] == 0:
        raise VerifyError("no program headers")
    return header


def parse_phdrs(handle, header):
    phdrs = []
    for index in range(header["e_phnum"]):
        offset = header["e_phoff"] + index * 56
        fields = struct.unpack("<IIQQQQQQ", read_exact(handle, offset, 56, "program header"))
        phdrs.append(
            {
                "p_type": fields[0],
                "p_flags": fields[1],
                "p_offset": fields[2],
                "p_vaddr": fields[3],
                "p_paddr": fields[4],
                "p_filesz": fields[5],
                "p_memsz": fields[6],
                "p_align": fields[7],
            }
        )
    return phdrs


def check_loads(phdrs):
    loads = [p for p in phdrs if p["p_type"] == PT_LOAD]
    if not loads:
        raise VerifyError("no PT_LOAD segments")
    for seg in loads:
        if seg["p_filesz"] > seg["p_memsz"]:
            raise VerifyError("PT_LOAD with p_filesz larger than p_memsz")
        if seg["p_paddr"] < MIN_PADDR:
            raise VerifyError(f"PT_LOAD paddr {seg['p_paddr']:#x} below {MIN_PADDR:#x}")
        end = seg["p_paddr"] + seg["p_memsz"]
        if end > FOUR_GIB:
            raise VerifyError(f"PT_LOAD ending at {end:#x} crosses 4 GiB")
    ordered = sorted((s for s in loads if s["p_memsz"] > 0), key=lambda s: s["p_paddr"])
    for earlier, later in zip(ordered, ordered[1:]):
        if earlier["p_paddr"] + earlier["p_memsz"] > later["p_paddr"]:
            raise VerifyError(
                f"PT_LOAD segments overlap: {earlier['p_paddr']:#x}+{earlier['p_memsz']:#x} "
                f"and {later['p_paddr']:#x}"
            )
    return loads


def iter_notes(handle, phdrs):
    for phdr in phdrs:
        if phdr["p_type"] != PT_NOTE or phdr["p_filesz"] == 0:
            continue
        data = read_exact(handle, phdr["p_offset"], phdr["p_filesz"], "PT_NOTE")
        cursor = 0
        while cursor + 12 <= len(data):
            namesz, descsz, ntype = struct.unpack_from("<III", data, cursor)
            cursor += 12
            name = data[cursor : cursor + namesz]
            cursor += (namesz + 3) & ~3
            desc = data[cursor : cursor + descsz]
            cursor += (descsz + 3) & ~3
            if len(name) != namesz or len(desc) != descsz:
                raise VerifyError("truncated ELF note")
            yield name.rstrip(b"\0"), ntype, desc


def find_pvh_entry(handle, phdrs):
    matches = []
    for name, ntype, desc in iter_notes(handle, phdrs):
        if name == b"Xen" and ntype == XEN_ELFNOTE_PHYS32_ENTRY:
            matches.append(desc)
    if not matches:
        raise VerifyError("no Xen note of type XEN_ELFNOTE_PHYS32_ENTRY (18) in any PT_NOTE")
    if len(matches) > 1:
        raise VerifyError("more than one XEN_ELFNOTE_PHYS32_ENTRY note")
    desc = matches[0]
    if len(desc) == 4:
        entry = struct.unpack("<I", desc)[0]
    elif len(desc) == 8:
        entry = struct.unpack("<Q", desc)[0]
    else:
        raise VerifyError(f"PHYS32_ENTRY descriptor is {len(desc)} bytes, expected 4 or 8")
    if entry == 0 or entry >= FOUR_GIB:
        raise VerifyError(f"PHYS32_ENTRY {entry:#x} is not a nonzero 32-bit address")
    return entry


def check_entry_in_exec_load(entry, loads):
    for seg in loads:
        start = seg["p_paddr"]
        end = start + seg["p_memsz"]
        if start <= entry < end:
            if not seg["p_flags"] & PF_X:
                raise VerifyError(f"PVH entry {entry:#x} lies in a non-executable PT_LOAD")
            if entry >= start + seg["p_filesz"]:
                raise VerifyError(f"PVH entry {entry:#x} lies in zero-filled memory of a PT_LOAD")
            return seg
    raise VerifyError(f"PVH entry {entry:#x} is outside every PT_LOAD segment")


def verify(path):
    with open(path, "rb") as handle:
        header = parse_header(handle)
        phdrs = parse_phdrs(handle, header)
        loads = check_loads(phdrs)
        entry = find_pvh_entry(handle, phdrs)
        segment = check_entry_in_exec_load(entry, loads)
    return {
        "verified": True,
        "elf_entry_vaddr": f"{header['e_entry']:#x}",
        "pvh_phys32_entry": f"{entry:#x}",
        "pvh_entry_segment_paddr": f"{segment['p_paddr']:#x}",
        "pt_load": [
            {
                "paddr": f"{s['p_paddr']:#x}",
                "vaddr": f"{s['p_vaddr']:#x}",
                "filesz": s["p_filesz"],
                "memsz": s["p_memsz"],
                "flags": s["p_flags"],
                "executable": bool(s["p_flags"] & PF_X),
            }
            for s in loads
        ],
        "min_paddr_required": f"{MIN_PADDR:#x}",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("vmlinux")
    parser.add_argument("--json", action="store_true", help="print a JSON report")
    args = parser.parse_args()
    try:
        report = verify(args.vmlinux)
    except (VerifyError, OSError, struct.error) as error:
        if args.json:
            print(json.dumps({"verified": False, "error": str(error)}, indent=2))
        else:
            print(f"verify-pvh: FAIL: {error}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(report, indent=2))
        return 0
    print("verify-pvh: OK")
    print(f"  ELF64 ET_EXEC EM_X86_64, e_entry={report['elf_entry_vaddr']}")
    for seg in report["pt_load"]:
        kind = "R-E" if seg["executable"] else "RW-"
        print(
            f"  PT_LOAD paddr={seg['paddr']} vaddr={seg['vaddr']} "
            f"filesz={seg['filesz']} memsz={seg['memsz']} {kind}"
        )
    print(
        f"  XEN_ELFNOTE_PHYS32_ENTRY = {report['pvh_phys32_entry']} "
        f"(inside executable PT_LOAD at {report['pvh_entry_segment_paddr']})"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
