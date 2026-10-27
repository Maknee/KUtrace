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

DRAM traffic on Zen 4. The encodings come from 7.2.13
`tools/perf/pmu-events/arch/x86/amdzen4/{memory-controller,data-fabric}.json`.
The `rdwrmask` field is `config:8-9` in `arch/x86/events/amd/uncore.c`.

| What | Event | Unit |
|---|---|---|
| DRAM reads, per channel | `amd_umc_N/event=0x0a,rdwrmask=0x1/` (config 0x10a, `umc_cas_cmd.rd`) | 64 B per count |
| DRAM writes, per channel | `amd_umc_N/event=0x0a,rdwrmask=0x2/` (config 0x20a, `umc_cas_cmd.wr`) | 64 B per count |
| DRAM reads via DF, CS k=0..11 | `amd_df/event=0x1f+0x40*k,umask=0x7fe/` (`local_processor_read_data_beats_csk`) | 64 B per count |
| DRAM writes via DF, CS k=0..11 | `amd_df/event=0x1f+0x40*k,umask=0x7ff/` | 64 B per count |
| L3 accesses / misses / hits | `amd_l3/event=0x04,umask=0xff/`, `umask=0x01`, `umask=0xfe` | lines |

An EPYC 9354P has 12 UMC instances (`amd_umc_0..11`). Sum over all of them,
for example:

```
EV=""; for i in $(seq 0 11); do EV="$EV,amd_umc_$i/event=0x0a,rdwrmask=0x1,name=rd$i/,amd_umc_$i/event=0x0a,rdwrmask=0x2,name=wr$i/"; done
sudo perf stat -a -x, -e ${EV#,} -- <workload> 2>&1 | awk -F, '$3~/^rd/{r+=$1} $3~/^wr/{w+=$1} END{print r*64/2^30" GiB read, "w*64/2^30" GiB written"}'
```

The perf 6.6.36 userspace knows the `amd_df/...` event names but not the UMC
names. Use the raw form above, or build `tools/perf` from the 7.2.13 tree.

Checked on clnode367 (EPYC 9354P, NPS4) running 7.2.13. Each 4 GiB streaming
pass measured about 4.03 GiB of UMC reads; the same pass with NT stores measured
about 4.0 GiB of UMC writes and close to 0 reads. The amd_df counts matched
within 1-2%.

DMA engines: PTDMA (`1022:1498`, Rome/Milan) and AE4DMA are not present on Genoa.
The Genoa CCP/PSP (`1022:14ca`) binds to `ccp` with the PSP/SEV function only
and creates no DMA channels. The Genoa SDXI functions (`1022:14dc`) have no
driver in 7.2.13. So `/sys/class/dma` is empty and `dmatest` has nothing to
run on.
