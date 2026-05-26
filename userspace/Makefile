# Theory OS userspace build
# Requires: x86_64-linux-gnu-gcc or x86_64-elf-gcc, binutils

BUILD_DIR := build
CC ?= gcc
LD ?= ld

CFLAGS := -nostdlib -static -fno-stack-protector
LDFLAGS := -nostdlib -static -Ttext=0x400000

.PHONY: all clean init sh ui busybox rootfs

all: init sh ui

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

init: $(BUILD_DIR)/init
sh: $(BUILD_DIR)/sh
ui: $(BUILD_DIR)/ui

$(BUILD_DIR)/init: init/init.S | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c init/init.S -o $(BUILD_DIR)/init.o
	$(LD) $(LDFLAGS) -o $@ $(BUILD_DIR)/init.o

$(BUILD_DIR)/sh: sh/sh.S | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c sh/sh.S -o $(BUILD_DIR)/sh.o
	$(LD) $(LDFLAGS) -o $@ $(BUILD_DIR)/sh.o

$(BUILD_DIR)/ui: ui/ui.S | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c ui/ui.S -o $(BUILD_DIR)/ui.o
	$(LD) $(LDFLAGS) -o $@ $(BUILD_DIR)/ui.o

# Optional: static busybox when source tree present
$(BUILD_DIR)/busybox: | $(BUILD_DIR)
	@if [ -d busybox ]; then \
		cd busybox && make defconfig && sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' .config && \
		make -j$$(nproc) && cp busybox ../$(BUILD_DIR)/busybox; \
	fi

busybox: $(BUILD_DIR)/busybox

rootfs: init sh ui
	@echo "Userspace binaries in $(BUILD_DIR)/"

clean:
	rm -rf $(BUILD_DIR)
