# Build and run Theory OS in QEMU on Windows.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
$Bash = "C:\tools\msys64\usr\bin\bash.exe"
$Limine = Join-Path $Root "build\limine\limine-binary"
$Iso = Join-Path $Root "build\theory.iso"
$Kernel = Join-Path $Root "target\x86_64-unknown-none\release\theory-kernel"

if (-not (Test-Path $Qemu)) {
    Write-Error "QEMU not found. Install with: choco install qemu -y"
}

Write-Host "Building kernel..."
Push-Location (Join-Path $Root "kernel")
cargo +nightly build --release -Z build-std=core,compiler_builtins,alloc --target x86_64-unknown-none
Pop-Location

if (-not (Test-Path (Join-Path $Limine "limine-bios-cd.bin"))) {
    Write-Host "Downloading Limine..."
    New-Item -ItemType Directory -Force -Path (Join-Path $Root "build\limine") | Out-Null
    $zip = Join-Path $Root "build\limine-binary.zip"
    Invoke-WebRequest -Uri "https://github.com/Limine-Bootloader/Limine/releases/download/v12.3.1/limine-binary.zip" -OutFile $zip
    Expand-Archive -Path $zip -DestinationPath (Join-Path $Root "build\limine") -Force
}

Write-Host "Creating ISO..."
& $Bash -lc @"
set -e
ROOT='$(($Root -replace '\\','/') -replace '^C:','/c')'
ISO_DIR=`"`$ROOT/build/iso`"
rm -f `"`$ROOT/build/theory.iso`"
rm -rf `"`$ISO_DIR`"
mkdir -p `"`$ISO_DIR/boot`" `"`$ISO_DIR/EFI/BOOT`"
cp `"`$ROOT/target/x86_64-unknown-none/release/theory-kernel`" `"`$ISO_DIR/boot/theory-kernel`"
cp `"`$ROOT/kernel/limine.conf`" `"`$ISO_DIR/boot/limine.conf`"
cp `"`$ROOT/build/limine/limine-binary/limine-bios-cd.bin`" `"`$ISO_DIR/boot/`"
cp `"`$ROOT/build/limine/limine-binary/limine-bios.sys`" `"`$ISO_DIR/boot/`"
cp `"`$ROOT/build/limine/limine-binary/limine-uefi-cd.bin`" `"`$ISO_DIR/boot/`"
cp `"`$ROOT/build/limine/limine-binary/BOOTX64.EFI`" `"`$ISO_DIR/EFI/BOOT/`"
xorriso -as mkisofs -R -r -J -b boot/limine-bios-cd.bin -no-emul-boot -boot-load-size 4 -boot-info-table \
  -hfsplus -apm-block-size 2048 --efi-boot boot/limine-uefi-cd.bin -efi-boot-part --efi-boot-image \
  --protective-msdos-label `"`$ISO_DIR`" -o `"`$ROOT/build/theory.iso`"
`"`$ROOT/build/limine/limine-binary/limine-tool-windows-x86/limine.exe`" bios-install `"`$ROOT/build/theory.iso`"
"@

Write-Host "Starting QEMU (SDL window)..."
Start-Process -FilePath $Qemu -ArgumentList @(
    "-machine", "pc",
    "-cdrom", $Iso,
    "-boot", "order=d",
    "-m", "512M",
    "-smp", "1",
    "-display", "sdl",
    "-device", "e1000,netdev=net0",
    "-netdev", "user,id=net0",
    "-serial", "file:build/qemu-serial.log"
)
Write-Host "QEMU launched. Look for the QEMU window on your desktop."
