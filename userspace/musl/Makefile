# Theory OS musl libc port (cross-build)

# Prerequisites:
#   git clone https://github.com/bminor/musl
#   x86_64-linux-musl cross compiler (or build musl-native)

MUSL_DIR ?= musl
BUILD_DIR := build
CROSS ?= x86_64-linux-musl

.PHONY: all musl clean

all: musl

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

musl: | $(BUILD_DIR)
	@if [ -d "$(MUSL_DIR)" ]; then \
		cd $(MUSL_DIR) && \
		CC=$(CROSS)-gcc CROSS_COMPILE=$(CROSS)- ./configure --prefix=$$(pwd)/../$(BUILD_DIR)/musl && \
		make -j$$(nproc) && make install; \
		echo "musl installed to $(BUILD_DIR)/musl"; \
	else \
		echo "Clone musl into userspace/musl to build"; \
	fi

clean:
	rm -rf $(BUILD_DIR)/musl
