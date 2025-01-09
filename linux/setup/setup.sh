#!/bin/bash


#https://davidaugustat.com/linux/how-to-compile-linux-kernel-on-ubuntu

sudo apt install build-essential libncurses-dev bison flex libssl-dev libelf-dev fakeroot dwarves -y

wget https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.6.36.tar.xz
tar xvf linux-6.6.36.tar.xz
cd linux-6.6.36

patch -p1 < ../kutrace_patch_file_6.6.36.txt 

cp -v /boot/config-$(uname -r) .config
yes "" | make localmodconfig

scripts/config --disable SYSTEM_TRUSTED_KEYS
scripts/config --disable SYSTEM_REVOCATION_KEYS
scripts/config --set-str CONFIG_SYSTEM_TRUSTED_KEYS ""
scripts/config --set-str CONFIG_SYSTEM_REVOCATION_KEYS ""
scripts/config --enable KEXEC
scripts/config --enable KEXEC_FILE
scripts/config --enable KUTRACE
scripts/config --disable NO_HZ_COMMON
scripts/config --disable NO_HZ_FULL
scripts/config --disable NO_HZ
scripts/config --disable NO_HZ_IDLE
scripts/config --enable HZ_PERIODIC
scripts/config --enable USB_STORAGE
scripts/config --enable SCSI
scripts/config --enable BLK_DEV_SD


yes "" | make localmodconfig

fakeroot make -j$(($(nproc) - 1))

cd tools/perf
make -j
cp perf ../../../postproc
cd ../..

sudo make modules_install
sudo make install

sudo apt-get install kexec-tools -y
# sudo kexec -l arch/x86/boot/bzImage --append="$(cat /proc/cmdline) modulepath=$(pwd)/lib/modules" --reuse-cmdline
# cat /sys/kernel/kexec_loaded
sudo kexec -l /boot/vmlinuz-6.6.36 --initrd=/boot/initrd.img-6.6.36 --append="$(cat /proc/cmdline)"
sudo kexec -e

cd module
make -j

sudo insmod kutrace_mod.ko tracemb=20 check=0
sudo rmmod kutrace_mod.ko

cd ../

cp postproc_changes/* postproc/

cd postproc
./build.sh


