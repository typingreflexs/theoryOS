#!/usr/bin/env bash
QEMU="/c/Program Files/qemu/qemu-system-x86_64.exe"
ISO="/c/Users/dogar/Downloads/THE/build/theory.iso"
OUT="/c/Users/dogar/Downloads/THE/build/qemu-serial-out.txt"
rm -f "$OUT"
timeout 15 "$QEMU" -cdrom "$ISO" -m 512M -display none -serial stdio -no-reboot > "$OUT" 2>&1 || true
wc -c "$OUT"
/usr/bin/head -c 2000 "$OUT" 2>/dev/null || cat "$OUT"
