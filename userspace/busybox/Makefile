# Busybox static build for Theory OS

BUSYBOX_DIR ?= busybox
BUILD_DIR := ../build
CROSS ?= x86_64-linux-musl

.PHONY: all clean

all:
	@if [ -d "$(BUSYBOX_DIR)" ]; then \
		cd $(BUSYBOX_DIR) && \
		make defconfig && \
		sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' .config && \
		sed -i 's/# CONFIG_FEATURE_SH_IS_NONE is not set/CONFIG_FEATURE_SH_IS_ASH=y/' .config 2>/dev/null || true && \
		make -j$$(nproc) CROSS_COMPILE=$(CROSS)- && \
		cp busybox $(BUILD_DIR)/busybox; \
	else \
		echo "Clone busybox into userspace/busybox to build"; \
	fi

clean:
	@if [ -d "$(BUSYBOX_DIR)" ]; then $(MAKE) -C $(BUSYBOX_DIR) distclean; fi
