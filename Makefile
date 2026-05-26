.PHONY: kernel iso run clean userspace test

test:
	cd kernel && cargo test --lib --target x86_64-pc-windows-msvc

KERNEL_ELF := target/x86_64-unknown-none/release/theory-kernel
ISO_DIR := build/iso
LIMINE_DIR := build/limine

userspace:
	$(MAKE) -C userspace

kernel: userspace
	cd kernel && cargo +nightly build --release -Z build-std=core,compiler_builtins,alloc --target x86_64-unknown-none

iso: kernel
	rm -f build/theory.iso
	rm -rf $(ISO_DIR)
	mkdir -p $(ISO_DIR)/boot $(ISO_DIR)/EFI/BOOT
	cp $(KERNEL_ELF) $(ISO_DIR)/boot/theory-kernel
	cp kernel/limine.conf $(ISO_DIR)/boot/
	cp $(LIMINE_DIR)/limine-bios-cd.bin $(LIMINE_DIR)/limine-bios.sys $(LIMINE_DIR)/limine-uefi-cd.bin $(ISO_DIR)/boot/
	cp $(LIMINE_DIR)/BOOTX64.EFI $(ISO_DIR)/EFI/BOOT/
	xorriso -as mkisofs -R -r -J -b boot/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		-hfsplus -apm-block-size 2048 \
		--efi-boot boot/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		$(ISO_DIR) -o build/theory.iso
	$(LIMINE_DIR)/limine bios-install build/theory.iso

run: iso
	qemu-system-x86_64 -machine pc -cdrom build/theory.iso -boot order=d -serial stdio -m 512M -smp 4 -display sdl

clean:
	cd kernel && cargo clean
	rm -rf build
