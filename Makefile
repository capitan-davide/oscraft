BUILD    := build

# OsCraft builds and runs on QEMU's virt machine with FreeBSD's userland,
# built from source (docs/setup.md), and nothing else. The QNX
# compatibility personality is opt-in: QNX=1 adds QNX 8.0's own programs
# and libraries, from the user's licensed QNX SDP, under /compat/qnx
# (images/qnx.build), and the QNX test programs, the oracle the QNX
# personality is measured against. The SDP is reached only through the
# environment its qnxsdp-env.sh sets (QNX_HOST, QNX_TARGET), which the
# targets that need it check first.
need-sdp = $(if $(QNX_HOST),,$(error QNX_HOST is not set: source your QNX SDP's qnxsdp-env.sh))$(if \
	$(QNX_TARGET),,$(error QNX_TARGET is not set: source your QNX SDP's qnxsdp-env.sh))

CLANG    ?= clang
LLD      ?= ld.lld

# Kernel: cargo profile "dev" (default) or "release".
KERNEL_TARGET := aarch64-unknown-none-softfloat
PROFILE       ?= dev
PROFILE_DIR   := $(if $(filter dev,$(PROFILE)),debug,$(PROFILE))
KERNEL_ELF    := target/$(KERNEL_TARGET)/$(PROFILE_DIR)/kernel
IMAGE         := $(BUILD)/oscraft.ifs

.PHONY: all kernel user qnx-tests image test-image qemu-build test-build image-env run debug mkifs startup qnx-image qnx-run qnx-test clean test clippy pwd-db

all: image

kernel:
	cargo build -p kernel --target $(KERNEL_TARGET) --profile $(PROFILE)

# Lints (`[workspace.lints]` in Cargo.toml) for the kernel's own target:
# the host target would reject its inline assembly and cfgs.
clippy:
	cargo clippy -p kernel --target $(KERNEL_TARGET)

# The two minimal Rust programs without a libc, packed into the image:
# hello, a native one, and qnxhello, one of the QNX personality (the
# overlay's).
USER_DIR := target/$(KERNEL_TARGET)/$(PROFILE_DIR)
user:
	cargo build -p hello -p qnxhello --target $(KERNEL_TARGET) --profile $(PROFILE)
	# hello is a native program: EI_OSABI = ELFOSABI_FREEBSD (9).
	printf '\011' | dd of=$(USER_DIR)/hello bs=1 seek=7 conv=notrunc status=none

# The userland. FreeBSD's source tree (a sibling checkout of the oscraft
# branch of stable/15) is cross-built for arm64 with its own
# tools/build/make.py, into FREEBSD_OBJ; its temporary tree is a complete
# sysroot (headers, crt files, every library, the loader). OsCraft's
# libsys (user/libsys, Rust) is emitted as one object, which FreeBSD's
# build takes into libc, libsys and the loader in place of its syscall
# stubs (OSCRAFT_LIBSYS_OBJ, lib/libsys/Makefile.sys). The target
# aarch64-unknown-freebsd has no prebuilt core on stable Rust, so libsys
# builds with a nightly and -Zbuild-std.
FREEBSD_SRC ?= $(abspath ../freebsd-src)
FREEBSD_OBJ ?= $(abspath ../freebsd-obj)
FREEBSD_TMP := $(FREEBSD_OBJ)$(FREEBSD_SRC)/arm64.aarch64/tmp
FREEBSD_DIR := $(BUILD)/freebsd
LIBSYS_OBJ := $(FREEBSD_DIR)/oscraft_libsys.o
LIBSYS_TOOLCHAIN ?= nightly-2026-08-17
FREEBSD_TRIPLE := aarch64-unknown-freebsd15.0
BCC := $(CLANG) --target=$(FREEBSD_TRIPLE) --sysroot=$(FREEBSD_TMP)
BCFLAGS := -O2 -g -fno-omit-frame-pointer -Wall
# FreeBSD's directories, in the order their libraries need each other.
FREEBSD_LIBS := lib/csu lib/libcompiler_rt lib/libsys lib/libc lib/libc_nonshared \
	libexec/rtld-elf lib/libgcc_eh lib/libgcc_s lib/libthr lib/librt lib/libutil \
	lib/libelf lib/libexecinfo lib/libkvm lib/libmemstat lib/libdevstat lib/libprocstat
# GNU make's MAKEFLAGS and MAKELEVEL in the environment confuse bmake.
FREEBSD_MAKE := env -u MAKEFLAGS -u MFLAGS -u MAKELEVEL MAKEOBJDIRPREFIX=$(FREEBSD_OBJ) PATH=$(FREEBSD_OBJ)/hostbin:$$PATH \
	python3 $(FREEBSD_SRC)/tools/build/make.py --cross-bindir=/usr/bin \
	TARGET=arm64 TARGET_ARCH=aarch64 -DWITHOUT_CLEAN DESTDIR=$(FREEBSD_TMP)

libsys:
	@mkdir -p $(FREEBSD_DIR)
	CARGO_PROFILE_RELEASE_LTO=fat cargo +$(LIBSYS_TOOLCHAIN) rustc --release \
	    -Zbuild-std=core --target aarch64-unknown-freebsd -p libsys \
	    -- --emit=obj=$(abspath $(LIBSYS_OBJ)) -C codegen-units=1

# The generated stubs: every system call libsys does not implement by hand.
libsys-stubs:
	python3 user/libsys/tools/gen-stubs.py $(FREEBSD_SRC) > user/libsys/src/stubs.rs

# FreeBSD's libraries with libsys inside, into the sysroot (the stages
# before them, bootstrap and cross tools and the headers, are run once by
# hand: tools/freebsd/README.md).
freebsd-libs: libsys
	cd $(FREEBSD_SRC) && $(FREEBSD_MAKE) buildenv \
	    BUILDENV_SHELL="env OSCRAFT_LIBSYS_OBJ=$(abspath $(LIBSYS_OBJ)) /bin/sh $(abspath tools/freebsd/build-libs.sh) $(FREEBSD_LIBS)"

# FreeBSD's network stack as a library, for netd.
bsdnet: $(FREEBSD_DIR)/libbsdnet.a
BSDLIB = cd $(FREEBSD_SRC) && $(FREEBSD_MAKE) buildenv \
	    BUILDENV_SHELL="env GLUE=$(3) EXCLUDE=$(4) /bin/sh $(abspath tools/freebsd/build-bsdlib.sh) $(1) $(FREEBSD_SRC) $(abspath $(FREEBSD_DIR)) $(abspath user/bsdkern/kern) $(FREEBSD_TMP)/usr/lib/libc.a $(2)"
$(FREEBSD_DIR)/libbsdnet.a: $(wildcard user/bsdkern/kern/* user/netd/kern/*) tools/freebsd/build-bsdlib.sh tools/freebsd/conf/OSCRAFT_NET
	@mkdir -p $(FREEBSD_DIR)
	$(call BSDLIB,bsdnet,'^(net|netinet|netinet6|netlink)/|^kern/(uipc_(socket|sockbuf|domain|mbuf|mbuf2|mbufhash|accf|debug)|kern_mbuf|sys_socket|subr_mbpool|subr_bufring)' bus_if.o device_if.o linker_if.o usb_if.o miibus_if.o pci_if.o virtio_if.o virtio_bus_if.o virtio_pci_if.o usbdevs.h miidevs.h,$(abspath user/netd/kern))

# FreeBSD's USB core and host controllers as a library, for usbd.
bsdusb: $(FREEBSD_DIR)/libbsdusb.a
# (Less usb_pf.c, packet capture, which needs a network stack.)
$(FREEBSD_DIR)/libbsdusb.a: $(wildcard user/bsdkern/kern/* user/usbd/kern/*) tools/freebsd/build-bsdlib.sh tools/freebsd/conf/OSCRAFT_USB
	@mkdir -p $(FREEBSD_DIR)
	$(call BSDLIB,bsdusb,'^dev/usb/|^dev/pci/pcib_support\.c' bus_if.o device_if.o linker_if.o usb_if.o pci_if.o pcib_if.o,$(abspath user/usbd/kern),'usb_pf\.c')

# FreeBSD's PCI bus, its FDT layer and the generic host bridge, for pcid.
bsdpci: $(FREEBSD_DIR)/libbsdpci.a
$(FREEBSD_DIR)/libbsdpci.a: $(wildcard user/bsdkern/kern/* user/pcid/kern/*) tools/freebsd/build-bsdlib.sh tools/freebsd/conf/OSCRAFT_PCI
	@mkdir -p $(FREEBSD_DIR)
	$(call BSDLIB,bsdpci,'^dev/pci/(pci|pci_pci|pci_subr|pci_user|pci_host_generic|pci_host_generic_fdt)\.c|^dev/ofw/(openfirm|ofw_fdt|ofw_bus_subr|ofw_subr|ofwbus|ofw_pci)\.c|^dev/pci/pcib_support\.c|^dev/fdt/(simplebus|fdt_common)\.c|^contrib/libfdt/' bus_if.o device_if.o linker_if.o pci_if.o pcib_if.o ofw_bus_if.o ofw_if.o msi_if.o pic_if.o,$(abspath user/pcid/kern))

# Modules of the bsdkern servers: shared objects of FreeBSD kernel C, built
# as the server's library is (tools/freebsd/build-bsdmod.sh), which
# kldload(2) loads into the server. netd's (user/netd/mods):
BSDMOD_DEPS := user/bsdkern/mods/module.c $(wildcard user/bsdkern/mods/include/*/*.h) tools/freebsd/build-bsdmod.sh
BSDMOD = cd $(FREEBSD_SRC) && $(FREEBSD_MAKE) buildenv BUILDENV_SHELL="env MODCFLAGS='$(3)' /bin/sh $(abspath tools/freebsd/build-bsdmod.sh) $(1) $(abspath $@) $(2)"
NETD_MODS := $(FREEBSD_DIR)/mods/pfil_test.ko $(FREEBSD_DIR)/mods/ipfw.ko
netd-mods: $(NETD_MODS)
# FreeBSD's ipfw, as sys/modules/ipfw/Makefile builds it.
IPFW_SRCS := $(addprefix $(FREEBSD_SRC)/sys/netpfil/ipfw/,ip_fw2.c ip_fw_pfil.c ip_fw_bpf.c ip_fw_dynamic.c \
	ip_fw_log.c ip_fw_eaction.c ip_fw_sockopt.c ip_fw_table.c ip_fw_table_algo.c ip_fw_iface.c ip_fw_table_value.c)
$(FREEBSD_DIR)/mods/ipfw.ko: $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(IPFW_SRCS),-DIPFIREWALL)
$(FREEBSD_DIR)/mods/pfil_test.ko: user/netd/mods/pfil_test.c $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(abspath $<))

# netd's USB: FreeBSD's USB core with bsdkern's client controller, over
# usbd (usb.ko), and the class drivers (if_cdce.ko: USB Ethernet, CDC ECM).
USB_CORE_SRCS := $(addprefix $(FREEBSD_SRC)/sys/dev/usb/,quirk/usb_quirk.c usb_busdma.c usb_core.c usb_debug.c \
	usb_dev.c usb_device.c usb_dynamic.c usb_error.c usb_generic.c usb_handle_request.c usb_hid.c usb_hub.c \
	usb_lookup.c usb_mbuf.c usb_msctest.c usb_parse.c usb_process.c usb_request.c usb_transfer.c usb_util.c \
	usb_pf.c controller/usb_controller.c)
$(FREEBSD_DIR)/mods/usb.ko: user/bsdkern/usb/usbclient.c user/bsdkern/kern/bsdkern_usb.h $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(USB_CORE_SRCS) $(abspath user/bsdkern/usb/usbclient.c))
# Shared by the adapters' drivers, loaded once, as FreeBSD splits them:
# the MII layer with the PHYs USB adapters have, and usb_ethernet.c.
$(FREEBSD_DIR)/mods/miibus.ko: $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(addprefix $(FREEBSD_SRC)/sys/dev/mii/,mii.c mii_physubr.c ukphy.c ukphy_subr.c rgephy.c smscphy.c))
$(FREEBSD_DIR)/mods/uether.ko: $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(FREEBSD_SRC)/sys/dev/usb/net/usb_ethernet.c)
# The adapters: CDC ECM, RNDIS, ASIX, Realtek, SMSC and Microchip.
USB_NET := cdce urndis axe axge ure smsc muge
$(FREEBSD_DIR)/mods/if_%.ko: $(FREEBSD_SRC)/sys/dev/usb/net/if_%.c $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$<)
NETD_USB_MODS := $(addprefix $(FREEBSD_DIR)/mods/,usb.ko miibus.ko uether.ko $(addsuffix .ko,$(addprefix if_,$(USB_NET))))
netd-mods: $(NETD_USB_MODS)

# netd's PCI: bsdkern's PCI client bus, over pcid (pci.ko), and the network
# device of QEMU's virt: FreeBSD's virtio, its PCI transport and vtnet, split
# as FreeBSD's own modules are.
VIRTIO_SRC := $(FREEBSD_SRC)/sys/dev/virtio
$(FREEBSD_DIR)/mods/pci.ko: user/bsdkern/kern/pciclient.c user/bsdkern/kern/bsdkern_pci.h $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(abspath user/bsdkern/kern/pciclient.c),-DBSDKERN_PCI_CLIENT -DBSDKERN_PCI_CLIENT_MODULE)
$(FREEBSD_DIR)/mods/virtio.ko: $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(addprefix $(VIRTIO_SRC)/,virtio.c virtqueue.c))
$(FREEBSD_DIR)/mods/virtio_pci.ko: $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(addprefix $(VIRTIO_SRC)/pci/,virtio_pci.c virtio_pci_modern.c virtio_pci_legacy.c))
$(FREEBSD_DIR)/mods/if_vtnet.ko: $(BSDMOD_DEPS) $(FREEBSD_DIR)/libbsdnet.a
	$(call BSDMOD,bsdnet,$(VIRTIO_SRC)/network/if_vtnet.c)
NETD_PCI_MODS := $(addprefix $(FREEBSD_DIR)/mods/,pci.ko virtio.ko virtio_pci.ko if_vtnet.ko)
netd-mods: $(NETD_PCI_MODS)

# FreeBSD's programs, linked statically, into the sysroot under
# oscraft/bin (not its bin, which FreeBSD's build environment has on its
# PATH, for the host's tools): the shell and what it needs.
FREEBSD_PROGS := lib/libnv lib/libipsec lib/libifconfig lib/libsbuf lib/libjail lib/libkiconv lib/libkvm lib/libxo lib/msun lib/libregex lib/libmd lib/ncurses/tinfo lib/libedit bin/sh bin/cat bin/echo bin/ls bin/cp bin/mv bin/rm bin/mkdir bin/rmdir bin/pwd bin/sleep bin/test bin/ln bin/chmod bin/date bin/kill \
	usr.bin/env bin/ps usr.bin/top sbin/ipfw sbin/kldload sbin/kldunload sbin/kldstat usr.sbin/pciconf sbin/sysctl sbin/ifconfig sbin/ping sbin/route usr.bin/netstat sbin/dmesg sbin/mount sbin/mount_msdosfs sbin/umount sbin/newfs_msdos usr.bin/which bin/df usr.bin/head usr.bin/logger usr.bin/tail usr.bin/wc usr.bin/grep usr.bin/sort usr.bin/uniq usr.bin/tee usr.bin/basename usr.bin/dirname usr.bin/printf usr.bin/true usr.bin/false usr.bin/touch usr.bin/id usr.bin/uname usr.bin/cut usr.bin/tr usr.bin/xargs usr.bin/seq usr.bin/yes usr.bin/hexdump usr.bin/getconf
FREEBSD_BIN := $(FREEBSD_TMP)/oscraft/bin
freebsd-progs:
	cd $(FREEBSD_SRC) && $(FREEBSD_MAKE) buildenv \
	    BUILDENV_SHELL="env OSCRAFT_LIBSYS_OBJ=$(abspath $(LIBSYS_OBJ)) OSCRAFT_BUILD_FLAGS='BINDIR=/oscraft/bin MK_CASPER=no MK_WIRELESS_SUPPORT=no MK_PF=no MK_NETGRAPH_SUPPORT=no MK_OFED=no MK_IPSEC_SUPPORT=no MK_NETLINK_SUPPORT=yes' /bin/sh $(abspath tools/freebsd/build-libs.sh) $(FREEBSD_PROGS)"

# Native servers: Rust programs for aarch64-unknown-freebsd (std from
# source, static), linked by clang against the sysroot through a wrapper.
NATIVE_CC := $(FREEBSD_DIR)/cc
NATIVE_BIN := target/aarch64-unknown-freebsd/release
NATIVE_CARGO := CARGO_TARGET_AARCH64_UNKNOWN_FREEBSD_LINKER=$(abspath $(NATIVE_CC)) \
	CARGO_TARGET_AARCH64_UNKNOWN_FREEBSD_RUSTFLAGS="-C target-feature=+crt-static" \
	cargo +$(LIBSYS_TOOLCHAIN) build --release -Zbuild-std=std,panic_abort --target aarch64-unknown-freebsd
# netd and pcid are dynamic, linked at a fixed address (FreeBSD's objects
# are kernel code, not position-independent), their symbols exported: their
# modules, shared objects, resolve against them (user/bsdkern/kern/link_dl.c).
NETD_CARGO := CARGO_TARGET_AARCH64_UNKNOWN_FREEBSD_LINKER=$(abspath $(NATIVE_CC)) \
	CARGO_TARGET_AARCH64_UNKNOWN_FREEBSD_RUSTFLAGS="-C relocation-model=static -C link-arg=-no-pie -C link-arg=-Wl,--export-dynamic" \
	cargo +$(LIBSYS_TOOLCHAIN) build --release -Zbuild-std=std,panic_abort --target aarch64-unknown-freebsd --target-dir target/netd-dyn
$(NATIVE_CC):
	@mkdir -p $(FREEBSD_DIR)
	@printf '#!/bin/sh\nexec $(CLANG) --target=$(FREEBSD_TRIPLE) --sysroot=$(FREEBSD_TMP) -fuse-ld=lld "$$@"\n' > $@ && chmod +x $@
# The servers are static: libsys and libc changing must relink them, which
# cargo can't see.
NATIVE_LIBC := $(FREEBSD_TMP)/usr/lib/libc.a
native: $(NATIVE_CC) $(FREEBSD_DIR)/libbsdnet.a $(FREEBSD_DIR)/libbsdusb.a $(FREEBSD_DIR)/libbsdpci.a
	@if [ -e target/aarch64-unknown-freebsd/release/build ] && \
	    [ $(NATIVE_LIBC) -nt target/aarch64-unknown-freebsd/release/syslogd ]; then \
		find target/aarch64-unknown-freebsd/release/build -path '*/fingerprint/bin-*' -delete; \
	fi
	$(NATIVE_CARGO) -p random -p pipe -p pty -p qnxd -p priochain -p syslogd -p diskd -p tmpfs -p bsdkern-tests -p usbd
	$(NETD_CARGO) -p netd -p pcid
	cp target/netd-dyn/aarch64-unknown-freebsd/release/netd target/netd-dyn/aarch64-unknown-freebsd/release/pcid $(NATIVE_BIN)/

# The terminfo entries FreeBSD's libedit and ncurses read, compiled from
# ncurses's source with the host's tic into the directory form
# (/usr/share/terminfo/<l>/<name>, lib/ncurses/tinfo's TERMINFO).
FREEBSD_TERMINFO := $(FREEBSD_DIR)/terminfo
TERMINFO_TERMS := xterm,xterm-256color,vt100,vt102,ansi,dumb,screen,linux
freebsd-terminfo:
	@mkdir -p $(FREEBSD_TERMINFO)
	tic -x -o $(FREEBSD_TERMINFO) -e $(TERMINFO_TERMS) $(FREEBSD_SRC)/contrib/ncurses/misc/terminfo.src

# The native test programs (tests/native): C programs built against the
# sysroot, as any program for OsCraft is. Each tests/native/<name>.c is
# /proc/boot/<name>-test, static; dyn-test and thr-dyn-test are libc.c
# and thr.c linked dynamically; hello-test has no libc at all (start1.c
# and libsys's object).
NATIVE_TESTS := $(BUILD)/native-tests
# The objects are kept between builds, not remade for every link.
.SECONDARY:
FREEBSD_CRT := $(FREEBSD_TMP)/usr/lib
TEST_PROGS := libc thr fork tty misc net usbd fs syslog pty ttyselect netintr net6 netstress pipe vtnet exec
LIBS_thr := -lthr
LIBS_misc := -lrt -lthr
LIBS_net := -lthr
LIBS_netstress := -lthr
CFLAGS_usbd := -Iuser/bsdkern/kern
STATIC_LINK = $(BCC) -nostdlib -static -fuse-ld=lld -o $@ $(FREEBSD_CRT)/crt1.o $(FREEBSD_CRT)/crti.o $(FREEBSD_CRT)/crtbegin.o \
	    $(1) -L$(FREEBSD_CRT) $(2) -lc -lsys -lcompiler_rt $(FREEBSD_CRT)/crtend.o $(FREEBSD_CRT)/crtn.o
PIE_LINK = $(BCC) -nostdlib -pie -fuse-ld=lld -o $@ $(FREEBSD_CRT)/Scrt1.o $(FREEBSD_CRT)/crti.o $(FREEBSD_CRT)/crtbeginS.o \
	    $(1) -L$(FREEBSD_CRT) $(2) -lc -lcompiler_rt $(FREEBSD_CRT)/crtendS.o $(FREEBSD_CRT)/crtn.o
$(NATIVE_TESTS)/%.o: tests/native/%.c
	@mkdir -p $(NATIVE_TESTS)
	$(BCC) $(BCFLAGS) $(CFLAGS_$*) -c $< -o $@
# Static: relinked when the libraries change (libsys is inside libc.a and
# libsys.a), or a test would go on running the system-call layer it was
# linked with.
$(NATIVE_TESTS)/%-test: $(NATIVE_TESTS)/%.o $(FREEBSD_CRT)/libc.a $(FREEBSD_CRT)/libsys.a $(FREEBSD_CRT)/libthr.a
	$(call STATIC_LINK,$<,$(LIBS_$*))
$(NATIVE_TESTS)/hello.o: tests/native/hello.c
	@mkdir -p $(NATIVE_TESTS)
	$(BCC) $(BCFLAGS) -fno-builtin -c $< -o $@
$(NATIVE_TESTS)/start1.o: tests/native/start1.c
	@mkdir -p $(NATIVE_TESTS)
	$(BCC) $(BCFLAGS) -fno-builtin -c $< -o $@
$(NATIVE_TESTS)/hello-test: $(NATIVE_TESTS)/hello.o $(NATIVE_TESTS)/start1.o
	$(BCC) -nostdlib -static -fuse-ld=lld -o $@ $(FREEBSD_CRT)/crt1.o $^ $(LIBSYS_OBJ)
$(NATIVE_TESTS)/dyn.o: tests/native/libc.c
	@mkdir -p $(NATIVE_TESTS)
	$(BCC) $(BCFLAGS) -fPIE -c $< -o $@
$(NATIVE_TESTS)/dyn-test: $(NATIVE_TESTS)/dyn.o
	$(call PIE_LINK,$<,)
$(NATIVE_TESTS)/thr-dyn.o: tests/native/thr.c
	@mkdir -p $(NATIVE_TESTS)
	$(BCC) $(BCFLAGS) -fPIE -c $< -o $@
$(NATIVE_TESTS)/thr-dyn-test: $(NATIVE_TESTS)/thr-dyn.o
	$(call PIE_LINK,$<,-lthr)
# A FAT image for the disk tests (fs-test, the mount test).
$(NATIVE_TESTS)/fat.img:
	@mkdir -p $(NATIVE_TESTS)
	dd if=/dev/zero of=$@ bs=1M count=8 status=none
	mkfs.fat -F 16 -S 512 -s 1 -n OSCRAFT $@ >/dev/null
	printf 'hello, disk\n' > $(NATIVE_TESTS)/hello.txt
	MTOOLS_SKIP_CHECK=1 mcopy -i $@ $(NATIVE_TESTS)/hello.txt ::/HELLO.TXT
	MTOOLS_SKIP_CHECK=1 mcopy -i $@ $(NATIVE_TESTS)/hello.txt '::/a long name.text'
native-tests: freebsd-terminfo $(addprefix $(NATIVE_TESTS)/,hello-test dyn-test thr-dyn-test fat.img $(addsuffix -test,$(TEST_PROGS)))

# The SDK (tools/sdk/README.md): the sysroot's headers, libraries and
# loader, the toolchain files and a manifest, assembled into build/sdk and
# packed; `make sdk-check` builds a C and a Rust program against it out
# of tree and runs them in QEMU.
SDK_VERSION := $(shell git describe --always --dirty 2>/dev/null || echo unknown)
SDK_NAME := oscraft-sdk-$(SDK_VERSION)-aarch64
SDK_DIR := $(BUILD)/sdk/$(SDK_NAME)
sdk:
	@echo "assembling $(SDK_DIR)"
	@rm -rf $(SDK_DIR) && mkdir -p $(SDK_DIR)/sysroot/usr $(SDK_DIR)/bin
	@cp -rL $(FREEBSD_TMP)/usr/include $(SDK_DIR)/sysroot/usr/
	@cp -rL $(FREEBSD_TMP)/lib $(FREEBSD_TMP)/libexec $(SDK_DIR)/sysroot/
	@mkdir -p $(SDK_DIR)/sysroot/usr/lib && cp -rL $(FREEBSD_TMP)/usr/lib/*.a $(FREEBSD_TMP)/usr/lib/*.o $(FREEBSD_TMP)/usr/lib/*.so* $(SDK_DIR)/sysroot/usr/lib/
	@cp tools/sdk/oscraft.cmake tools/sdk/clang.cfg tools/sdk/cargo-config.toml tools/sdk/README.md $(SDK_DIR)/
	@cp tools/sdk/oscraft-cc tools/sdk/oscraft-c++ $(SDK_DIR)/bin/
	@printf 'oscraft %s\nfreebsd %s + tools/freebsd/patches\nbuilt %s\n' "$(SDK_VERSION)" "$$(cat tools/freebsd/UPSTREAM_COMMIT)" "$$(date -u +%Y-%m-%dT%H:%MZ)" > $(SDK_DIR)/MANIFEST
	@cd $(BUILD)/sdk && tar --zstd -cf $(SDK_NAME).tar.zst $(SDK_NAME)
	@echo "SDK: $(BUILD)/sdk/$(SDK_NAME).tar.zst"

sdk-check: sdk
	tools/sdk/check.sh $(SDK_DIR) $(LIBSYS_TOOLCHAIN)

# One-time setup of FreeBSD's source and build system (docs/setup.md), and
# a check of everything the build and the tests need on this host.
freebsd-bootstrap:
	tools/freebsd/bootstrap.sh $(FREEBSD_SRC) $(FREEBSD_OBJ)

doctor:
	@FREEBSD_SRC=$(FREEBSD_SRC) FREEBSD_OBJ=$(FREEBSD_OBJ) tools/doctor.sh

.PHONY: libsys libsys-stubs bsdnet bsdusb bsdpci netd-mods freebsd-libs freebsd-progs freebsd-terminfo native-tests freebsd-bootstrap doctor sdk sdk-check native

# The QNX test programs (tests/qnx), the oracle of the QNX personality,
# built with the SDP's GCC against QNX's libc (QNX=1 only).
QNX_TESTS    := $(BUILD)/qnx-tests
QNX_CC       := $(QNX_HOST)/usr/bin/ntoaarch64-gcc
QNX_TEST_SRCS := $(wildcard tests/qnx/*.c)
# Every program is tests/qnx/<name>.c, dynamic, -O2, but the few below.
QNX_PROGS := $(filter-out hello sysinfo plsexectest interptest netdtest tcprelay dbgtest,$(basename $(notdir $(QNX_TEST_SRCS))))
qnx-tests:
	$(need-sdp)
	@mkdir -p $(QNX_TESTS)
	$(QNX_CC) -static -O2 -o $(QNX_TESTS)/hello-static tests/qnx/hello.c
	$(QNX_CC) -static -O2 -o $(QNX_TESTS)/sysinfo-static tests/qnx/sysinfo.c
	$(QNX_CC) -O2 -o $(QNX_TESTS)/hello-dyn tests/qnx/hello.c
	$(QNX_CC) -static -O2 -Wl,-e,plsexec_start -o $(QNX_TESTS)/plsexectest tests/qnx/plsexectest.c
	$(QNX_CC) -O2 -Wl,--dynamic-linker=/usr/lib/ld-oscraft.so.1 -o $(QNX_TESTS)/interptest tests/qnx/interptest.c
	$(QNX_CC) -O2 -o $(QNX_TESTS)/netdtest tests/qnx/netdtest.c -lsocket
	$(QNX_CC) -O2 -o $(QNX_TESTS)/tcprelay tests/qnx/tcprelay.c -lsocket
	$(QNX_CC) -O0 -g -o $(QNX_TESTS)/dbgtest tests/qnx/dbgtest.c
	for p in $(QNX_PROGS); do $(QNX_CC) -O2 -o $(QNX_TESTS)/$$p tests/qnx/$$p.c || exit 1; done

# Host tools: mkifs and dumpifs.
mkifs:
	cargo build -p mkifs --release

QNX_IMG  := $(BUILD)/qnx-reference.ifs

# sshd's host key: made once per build tree, not kept in the repository.
SSH_HOST_KEY := $(BUILD)/ssh_host_ed25519_key

# The startup: OsCraft's own (startup/boards/qemu-virt), a static library
# partially linked into the relocatable object mkifs links at the image's
# address, as QNX ships its startups; put in a root mkifs searches first
# (before $QNX_TARGET, when the SDP is there).
STARTUP_ROOT := $(abspath $(BUILD)/startup-root)
startup:
	cd startup && cargo build -p startup-qemu-virt --target aarch64-unknown-none-softfloat --release
	@mkdir -p $(STARTUP_ROOT)/aarch64le/boot/sys
	$(LLD) -r -u _start -T startup/startup.ld -o $(STARTUP_ROOT)/aarch64le/boot/sys/startup-qemu-virt \
	    startup/target/aarch64-unknown-none-softfloat/release/libstartup_qemu_virt.a

# The boot images. images/qemu-virt.build is the system: the startup, the
# kernel, the native servers and FreeBSD's userland, with FreeBSD's sh on
# the console; `make run` boots it as it is (build/oscraft.ifs). QNX=1
# appends the QNX compatibility overlay, images/qnx.build (QNX's programs
# and libraries under /compat/qnx, from the SDP).
#
# `make test` boots the same system with the test programs added
# (build/oscraft-test.ifs): tests/native/image.build, and with QNX=1
# tests/qnx/image.build, the QNX test programs with QNX's shell on its
# serial driver as the console, in place of the system's (the part of
# images/qemu-virt.build between its CONSOLE-BEGIN and CONSOLE-END lines is left out).
QNX ?= 0
ifeq ($(QNX),1)
IMAGE_PARTS := images/qemu-virt.build images/qnx.build
TEST_PARTS := images/qemu-virt.build tests/native/image.build images/qnx.build tests/qnx/image.build
TEST_CONSOLE_SED := -e '/^\# CONSOLE-BEGIN$$/,/^\# CONSOLE-END$$/d'
IMAGE_QNX_DEPS := qnx-tests
TEST_FLAGS := --qnx
else
IMAGE_PARTS := images/qemu-virt.build
TEST_PARTS := images/qemu-virt.build tests/native/image.build
TEST_CONSOLE_SED :=
IMAGE_QNX_DEPS :=
TEST_FLAGS :=
endif
TEST_IMAGE := $(BUILD)/oscraft-test.ifs
PWD_DB := $(BUILD)/pwd
OSCRAFT_IMAGE_ENV := OSCRAFT_KERNEL=$(KERNEL_ELF) OSCRAFT_USER=$(USER_DIR) OSCRAFT_QNX_TESTS=$(QNX_TESTS) \
	OSCRAFT_NATIVE_TESTS=$(NATIVE_TESTS) OSCRAFT_FREEBSD_SYSROOT=$(FREEBSD_TMP) OSCRAFT_FREEBSD_BIN=$(FREEBSD_BIN) \
	OSCRAFT_FREEBSD_TERMINFO=$(FREEBSD_TERMINFO) OSCRAFT_NATIVE=$(NATIVE_BIN) OSCRAFT_FREEBSD_DIR=$(abspath $(FREEBSD_DIR)) OSCRAFT_FREEBSD_SRC=$(abspath $(FREEBSD_SRC)) \
	OSCRAFT_SSH_KEY=$(SSH_HOST_KEY) OSCRAFT_PWD_DB=$(PWD_DB)
OSCRAFT_MKIFS = $(OSCRAFT_IMAGE_ENV) target/release/mkifs -r $(STARTUP_ROOT)
# The build files' variables, for tools/qnx/probe.py.
image-env:
	@echo $(OSCRAFT_IMAGE_ENV)

# FreeBSD's password databases, with the tool FreeBSD's build made for
# the host. -i: not installed.
pwd-db:
	@mkdir -p $(PWD_DB)
	cp images/master.passwd $(PWD_DB)/master.passwd
	$(FREEBSD_TMP)/legacy/bin/pwd_mkdb -i -d $(abspath $(PWD_DB)) $(abspath $(PWD_DB))/master.passwd

$(SSH_HOST_KEY):
	@mkdir -p $(BUILD)
	ssh-keygen -q -t ed25519 -N '' -C oscraft -f $@

# KDUMP=1: QNX's kdumper before the kernel, so that a kernel crash writes a
# compressed dump to the console (uuencoded kdump.elf.gz, for kdserver and
# gdb; tools/kdump). kdumper is the SDP's.
ifeq ($(KDUMP),1)
KDUMP_SED := -e 's|^\( *\)\$${OSCRAFT_KERNEL}|\1kdumper -C\n&|'
need-kdump = $(need-sdp)
endif

# SELFTEST=1: the kernel runs its own self-tests at boot (OSCRAFT_SELFTEST=1
# in its environment; off by default, as procnto has none).
ifeq ($(SELFTEST),1)
SELFTEST_SED := -e 's|^\( *\)PATH=/proc/boot|\1OSCRAFT_SELFTEST=1\n&|'
endif

IMAGE_DEPS := kernel user native netd-mods startup pwd-db mkifs $(SSH_HOST_KEY)
image: $(IMAGE_DEPS) qemu-build
	$(if $(filter 1,$(QNX)),$(need-sdp))$(need-kdump)
	$(OSCRAFT_MKIFS) $(BUILD)/oscraft-qemu.build $(IMAGE)

# The system with the test programs, for `make test`.
test-image: $(IMAGE_DEPS) $(IMAGE_QNX_DEPS) native-tests test-build
	$(if $(filter 1,$(QNX)),$(need-sdp))$(need-kdump)
	$(OSCRAFT_MKIFS) $(BUILD)/oscraft-test.build $(TEST_IMAGE)

# The build files alone. IMAGE_EXTRA names a file of extra entries appended
# to the system's (the SDK check adds its programs so). tools/qnx/probe.py
# starts from the tests' (`make QNX=1 test-build`), so that a probe runs
# the system `make test QNX=1` measures.
qemu-build:
	@mkdir -p $(BUILD)
	cat $(IMAGE_PARTS) | sed -e '' $(KDUMP_SED) $(SELFTEST_SED) > $(BUILD)/oscraft-qemu.build
	$(if $(IMAGE_EXTRA),cat $(IMAGE_EXTRA) >> $(BUILD)/oscraft-qemu.build)
test-build:
	@mkdir -p $(BUILD)
	cat $(TEST_PARTS) | sed -e '' $(TEST_CONSOLE_SED) $(KDUMP_SED) $(SELFTEST_SED) > $(BUILD)/oscraft-test.build

# QNX 8.0's own kernel on the same machine, for comparison: procnto and
# QNX's programs from the SDP, booted by OsCraft's startup
# (tests/qnx/reference.build), with the QNX test programs. `make qnx-test`
# runs the QNX tests on it: what the QNX personality is measured against.
qnx-image: mkifs startup qnx-tests
	$(need-sdp)
	@mkdir -p $(BUILD)/sym
	OSCRAFT_QNX_TESTS=$(QNX_TESTS) target/release/mkifs -o $(BUILD)/sym \
		-r $(STARTUP_ROOT) tests/qnx/reference.build $(QNX_IMG)

qnx-run: qnx-image
	tools/qemu/run.sh $(QNX_IMG)

qnx-test: qnx-image
	python3 -I -u tools/test/run-tests.py $(if $(STOP),--stop) --procnto $(QNX_IMG) tests/target.tests $(TESTS)

# Boot the image in QEMU and run the target tests (tests/target.tests,
# tools/test/run-tests.py): the native ones, and with QNX=1 the QNX
# personality's too. TESTS=name... runs only those; STOP=1 stops at the
# first failure.
test: test-image
	python3 -I -u tools/test/run-tests.py $(if $(STOP),--stop) $(TEST_FLAGS) $(TEST_IMAGE) tests/target.tests $(TESTS)

# Boot the image in QEMU; console on stdio (Ctrl-A X to quit).
run: image
	tools/qemu/run.sh $(IMAGE)

# As run, but wait for gdb: gdb -ex 'target remote :1234' $(KERNEL_ELF)
debug: image
	tools/qemu/run.sh --gdb $(IMAGE)

clean:
	rm -rf $(BUILD)
	cargo clean
	cd startup && cargo clean
