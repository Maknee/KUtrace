# KUtrace on Linux 7.2.13

Linux 6.6.36 cannot expose the AMD memory-controller PMU (`amd_umc_*` was added
in 6.7). This directory can now also build a KUtrace kernel based on 7.2.13. That
kernel has `amd_df`, `amd_l3` and `amd_umc_*` built in, along with the AMD/Intel
PMU and DMA-engine drivers.

Files:
- `kutrace_patch_file_7.2.13.txt`: the 6.6.36 KUtrace patch ported to 7.2.13.
  The hooks are unchanged. The 64-bit syscall hook moved from
  `arch/x86/entry/common.c` to `arch/x86/entry/syscall_64.c`, because upstream
  moved the code there.
- `setup.sh`: downloads 7.2.13, applies the patch, and configures the kernel
  (see the comments in it), then builds and installs it.
- `kutrace_patch_file_6.6.36.txt` is unchanged. To build 6.6.36 again, use the
  `setup.sh` from master.

## Build

```
cd linux/setup
./setup.sh          # or run its steps up to "fakeroot make" by hand
```

The script runs `make localmodconfig` against the modules loaded on the
*build host*. Build on a node with the same hardware and services as the
target. The PMU, DMA-engine, iptables-legacy and inet_diag options are forced to
`=y` after the last localmodconfig, so localmodconfig can't drop them.

If `setup.sh` is not used to install, copy the kernel to the target by hand.
`make bindeb-pkg` needs `debhelper`, so this uses a tar package instead:

```
make -j$(nproc) tarzst-pkg          # creates tar-install/{boot,lib/modules/7.2.13}
# on the target node:
sudo install -o root -g root -m 0644 boot/{config,System.map,vmlinuz}-7.2.13 /boot/
sudo cp -r --no-preserve=ownership lib/modules/7.2.13 /lib/modules/
sudo chown -R root:root /lib/modules/7.2.13
sudo depmod -a 7.2.13
sudo update-initramfs -c -k 7.2.13
sudo update-grub
```

Build the KUtrace module against the tree:
`make -C <linux-7.2.13> M=$PWD/module modules`.

## Boot once (keep 6.6.36 as the fallback)

```
grep -E '^GRUB_DEFAULT' /etc/default/grub      # must name an entry that really exists in grub.cfg
sudo grub-reboot "Advanced options for Ubuntu>Ubuntu, with Linux 7.2.13"
sudo reboot
```

GRUB menu titles are case-sensitive. If `GRUB_DEFAULT` does not match a
title in `/boot/grub/grub.cfg` exactly, GRUB boots entry 0. Entry 0 is the
newest kernel, which is now 7.2.13. In that case a failed boot does *not* fall
back to 6.6.36.

## After boot: re-mount /ssd

`/ssd` is a CloudLab temporary filesystem (xfs on the NVMe data disk). Mount it
again after every boot if it is not mounted already. Mount it by UUID, because
NVMe device names can change between boots. Never run `mkfs` or `wipefs` on it.

```
findmnt /ssd || sudo mount UUID=<uuid from /etc/fstab or lsblk -f> /ssd
ls /ssd
```

## Verify

```
uname -r                                   # 7.2.13
ls /sys/bus/event_source/devices | grep -E 'amd_df|amd_l3|amd_umc|ibs|power'
perf list pmu | grep -iE 'umc|amd_df|amd_l3'
rdma link
sudo insmod module/kutrace_mod.ko tracemb=40 check=0 && lsmod | grep kutrace && sudo rmmod kutrace_mod
```

DRAM traffic per memory channel on Zen 4 (encodings from
`tools/perf/pmu-events/arch/x86/amdzen4/memory-controller.json`; the
`rdwrmask` field is `config:8-9` in `arch/x86/events/amd/uncore.c`):

```
# CAS commands for reads / writes; each one is 64 bytes
sudo perf stat -a -e amd_umc_0/event=0x0a,rdwrmask=0x1/,amd_umc_0/event=0x0a,rdwrmask=0x2/ -- <workload>
sudo perf stat -a -M umc_mem_read_bandwidth,umc_mem_write_bandwidth -- <workload>
```

To get the system total, sum the counts over every `amd_umc_N` instance.

The DMA engines (AMD PTDMA/AE4DMA, Intel IOAT/IDXD) are built in, and
`dmatest` is built as a module. Run `ls /sys/class/dma` to see the channels.
