"""Emulate only the observed dense setter; never start the target executable.

Requires the existing local Unicorn package. No filesystem or network API is
available to the emulated machine code. Compares the entire output allocation
with the compiled Rust example, including every untouched byte.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess

from unicorn import Uc, UC_ARCH_X86, UC_MODE_64, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_RCX, UC_X86_REG_RDX, UC_X86_REG_R8, UC_X86_REG_RSP, UC_X86_REG_RIP


EXPECTED_SHA256 = "cf96adee1525e048de162c2dc4914e4bb1de8943b1e8e045c215f40105be2b53"
ENTRY = 0x140CE83C0
END = 0x140CE84CD
DATA = 0x04000000
STACK = 0x05000000
STOP = 0x06000000


def main():
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", type=Path, default=root.parent.parent / "Air奥奇传说1.0.3.exe")
    parser.add_argument("--probe", type=Path, default=root.parent / "target/release/examples/dense_vectors.exe")
    parser.add_argument("--report", type=Path, default=root.parent / "evidence/dense-machine-verification.json")
    args = parser.parse_args()
    source = args.target.resolve()
    data = source.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if digest != EXPECTED_SHA256:
        raise ValueError("Target does not match the evidence-bound build")

    pe = struct.unpack_from("<I", data, 0x3C)[0]
    count = struct.unpack_from("<H", data, pe + 6)[0]
    optional = pe + 24
    image_base = struct.unpack_from("<Q", data, optional + 24)[0]
    image_size = struct.unpack_from("<I", data, optional + 56)[0]
    sections = optional + struct.unpack_from("<H", data, pe + 20)[0]
    emu = Uc(UC_ARCH_X86, UC_MODE_64)
    emu.mem_map(image_base, (image_size + 4095) & ~4095)
    for i in range(count):
        _, rva, size, offset = struct.unpack_from("<IIII", data, sections + i * 40 + 8)
        if size:
            emu.mem_write(image_base + rva, data[offset:offset + size])
    for address in [DATA, STACK, STOP]:
        emu.mem_map(address, 0x10000)

    def guard(machine, address, size, _):
        if not ENTRY <= address < END:
            raise RuntimeError(f"Execution escaped selected function: {address:#x}")
        opcode = bytes(machine.mem_read(address, size))
        if opcode[0] == 0xE8 or opcode[:2] in (b"\x0f\x05", b"\x0f\x34"):
            raise RuntimeError("Selected valid-input path attempted a call/syscall")

    emu.hook_add(UC_HOOK_CODE, guard)
    initial = emu.context_save()
    blocks = DATA + 0x1000
    raw_value = DATA + 0x2000
    sentinel_block = bytes(64) + bytes([255]) * 8
    probe = args.probe.resolve()
    rust_run = subprocess.run([str(probe)], check=True, capture_output=True, text=True)
    lines = rust_run.stdout.splitlines()
    seen = set()
    for line in lines:
        fields = line.split("\t")
        if len(fields) != 4:
            raise ValueError("Malformed Rust probe output")
        discriminant = int(fields[0], 16)
        payload = int(fields[1], 16)
        index = int(fields[2])
        rust_blocks = bytes.fromhex(fields[3])
        if discriminant not in list(range(0x3A)) + [0xFFFFFFFFFFFFFFFF] or index not in [0, 7, 8]:
            raise ValueError("Input outside audited valid path")
        seen.add((discriminant, payload, index))
        emu.context_restore(initial)
        emu.mem_write(DATA, struct.pack("<QQQQ", 0, blocks, 2, 9))
        emu.mem_write(raw_value, struct.pack("<QQ", discriminant, payload))
        before = sentinel_block * 2
        emu.mem_write(blocks, before)
        emu.mem_write(STACK + 0xFFF0, struct.pack("<Q", STOP))
        emu.reg_write(UC_X86_REG_RCX, DATA)
        emu.reg_write(UC_X86_REG_RDX, index)
        emu.reg_write(UC_X86_REG_R8, raw_value)
        emu.reg_write(UC_X86_REG_RSP, STACK + 0xFFF0)
        emu.emu_start(ENTRY, STOP, timeout=1000000, count=256)
        if emu.reg_read(UC_X86_REG_RIP) != STOP:
            raise RuntimeError("Selected function did not return within the instruction/time limit")
        after = bytes(emu.mem_read(blocks, 144))
        lane = index & 7
        base = (index >> 3) * 72
        if after != rust_blocks:
            raise AssertionError(f"Rust/native mismatch for d={discriminant:x}, p={payload:x}, i={index}")
        # Ensure machine code did not alter any other element.
        unchanged = bytearray(after)
        unchanged[base + lane * 8:base + lane * 8 + 8] = bytes(8)
        unchanged[base + 64 + lane] = 255
        if bytes(unchanged) != before:
            raise AssertionError("Unexpected neighbor write")
    required = {(d, p, i) for d in list(range(0x3A)) + [0xFFFFFFFFFFFFFFFF]
                for p in [0, 0xFEDCBA98765432A5, 0xFFFFFFFFFFFFFFFF] for i in [0, 7, 8]}
    if seen != required or len(lines) != len(required):
        raise AssertionError("Rust probe did not produce the complete 531-case matrix")
    report = {
        "target_sha256": digest,
        "function_va": hex(ENTRY),
        "end_exclusive": hex(END),
        "method": "isolated Unicorn x86-64 instruction emulation, with escape/call guard",
        "test_cases": len(lines),
        "passed": True,
        "comparison": "all 144 allocation bytes for each case against compiled Rust output",
        "neighbor_writes": False,
        "rust_probe_sha256": hashlib.sha256(probe.read_bytes()).hexdigest(),
        "rust_output_sha256": hashlib.sha256(rust_run.stdout.encode()).hexdigest(),
        "limitations": [
            "Only the dense valid-index setter branch was emulated.",
            "This is not a Windows app launch or a complete runtime-equivalence test.",
            "Invalid representations, panic paths, allocation, GC and integration are not tested.",
        ],
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(
        json.dumps(report, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(report))


if __name__ == "__main__":
    main()
