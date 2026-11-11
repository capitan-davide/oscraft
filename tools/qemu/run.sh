#!/usr/bin/env bash
# Boot an OsCraft image in QEMU's virt machine: GICv2, 4 Cortex-A72s, 2 GB.
# The image is QEMU's -kernel, a raw image QEMU loads at 0x40080000 and
# enters with the device tree it made in x0, as startup-qemu-virt expects.
# The PL011 is the console (SERIAL1); PCIe has a virtio network device, and
# QEMU's xhci with a USB Ethernet adapter on it.
#
# Usage: run.sh [--gdb] [--log <file>] <image> [extra qemu args...]
#   --gdb         wait for a debugger on tcp::1234 before running
#   --log <file>  write QEMU's interrupt/guest-error trace to <file>
#
# Environment:
#   SERIAL1  QEMU -serial spec for the PL011, the console (default: mon:stdio)
#   VIRTIONET 1 (the default): a virtio network device on PCIe, netd's vtnet0
#            (pci.ko, virtio.ko, virtio_pci.ko, if_vtnet.ko), on QEMU's
#            user-mode network, which reaches the host's: the guest is
#            10.0.2.15/24, the gateway 10.0.2.2, the name server 10.0.2.3.
#            Modern virtio only (disable-legacy): the legacy interface is
#            an I/O port window, which pcid's clients don't get. 0: none
#   USBNET   1 (the default): a USB Ethernet adapter (CDC ECM) on QEMU's
#            xhci, on QEMU's user-mode network, which reaches the host's;
#            0: none
set -euo pipefail

qemu_args=()
if [[ ${VIRTIONET:-1} == 1 ]]; then
    qemu_args+=(-netdev user,id=net0 -device virtio-net-pci,netdev=net0,disable-legacy=on)
fi
if [[ ${USBNET:-1} == 1 ]]; then
    qemu_args+=(-device qemu-xhci,id=xhci -netdev user,id=usbnet -device usb-net,netdev=usbnet)
fi
while [[ $# -gt 0 ]]; do
    case $1 in
        --gdb) qemu_args+=(-s -S); shift ;;
        --log) qemu_args+=(-d int,guest_errors,unimp -D "$2"); shift 2 ;;
        *) break ;;
    esac
done
image=${1:?usage: run.sh [--gdb] [--log <file>] <image> [qemu args...]}
shift

# ANCHOR: qemu_exec
exec qemu-system-aarch64 \
    -M virt,gic-version=2 \
    -cpu cortex-a72 -smp 4 -m 2G \
    -kernel "$image" \
    -serial "${SERIAL1:-mon:stdio}" \
    -display none \
    "${qemu_args[@]}" \
    "$@"
# ANCHOR_END: qemu_exec
