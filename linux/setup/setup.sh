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
scripts/config --enable XFS_FS
scripts/config --enable CONFIG_X86_MSR

# Enable nftables subsystem
scripts/config --enable NETFILTER_XTABLES
scripts/config --enable NF_TABLES

# Enable core nftables features
scripts/config --enable NF_TABLES_SET
scripts/config --enable NF_TABLES_INET
scripts/config --enable NF_TABLES_NETDEV
scripts/config --enable NFT_NUMGEN
scripts/config --enable NFT_HASH
scripts/config --enable NFT_COUNTER
scripts/config --enable NFT_OBJREF

# Enable protocol support
scripts/config --enable NF_TABLES_IPV4
scripts/config --enable NF_TABLES_IPV6
scripts/config --enable NF_TABLES_ARP
scripts/config --enable NF_TABLES_BRIDGE

# Enable NAT and connection tracking
scripts/config --enable NFT_CT
scripts/config --enable NFT_FLOW_OFFLOAD
scripts/config --enable NFT_NAT
scripts/config --enable NFT_MASQ
scripts/config --enable NFT_REDIR
scripts/config --enable NFT_TPROXY

# Enable filtering and matching
scripts/config --enable NFT_QUOTA
scripts/config --enable NFT_LIMIT
scripts/config --enable NFT_REJECT
scripts/config --enable NFT_META
scripts/config --enable NFT_SOCKET
scripts/config --enable NFT_OSF
scripts/config --enable NFT_XFRM

# Enable logging
scripts/config --enable NFT_LOG

# Enable stateful inspection
scripts/config --enable NF_CONNTRACK
scripts/config --enable NF_CONNTRACK_EVENTS
scripts/config --enable NF_CONNTRACK_TIMEOUT
scripts/config --enable NF_CONNTRACK_TIMESTAMP

scripts/config --enable BRIDGE
scripts/config --enable BRIDGE_NETFILTER
scripts/config --enable OVERLAY_FS

scripts/config --enable IP_NF_IPTABLES
scripts/config --enable IP_NF_FILTER
scripts/config --enable IP_NF_NAT
scripts/config --enable IP_NF_MANGLE
scripts/config --enable IP_NF_TARGET_MASQUERADE
scripts/config --enable IP_NF_TARGET_REDIRECT

# Enable iptables match modules needed by Docker
scripts/config --enable NETFILTER_XT_MATCH_ADDRTYPE
scripts/config --enable NETFILTER_XT_MATCH_CONNTRACK
scripts/config --enable NETFILTER_XT_MATCH_STATE

scripts/config --module CEPH_FS
scripts/config --module CEPH_FSCACHE
scripts/config --module CEPH_FS_POSIX_ACL
scripts/config --module LIBCEPH
scripts/config --module CEPH_LIB
scripts/config --module CEPH_LIB_USE_DNS_RESOLVER

# # Enable additional modules that Docker might use
# scripts/config --enable NETFILTER_XT_MATCH_IPVS
# scripts/config --enable IP_VS
# scripts/config --enable VXLAN
# scripts/config --enable IPVLAN
# scripts/config --enable MACVLAN

scripts/config --enable VETH
scripts/config --enable IP_NF_FILTER
scripts/config --enable IP_NF_MANGLE
scripts/config --enable IP_NF_TARGET_MASQUERADE
scripts/config --enable IP6_NF_FILTER
scripts/config --enable IP6_NF_MANGLE
scripts/config --enable IP6_NF_TARGET_MASQUERADE
scripts/config --enable NETFILTER_XT_MATCH_ADDRTYPE
scripts/config --enable NETFILTER_XT_MATCH_IPVS
scripts/config --enable NETFILTER_XT_MARK
scripts/config --enable IP_NF_RAW
scripts/config --enable IP_NF_NAT
scripts/config --enable IP6_NF_RAW
scripts/config --enable IP6_NF_NAT
scripts/config --enable MEMCG_SWAP
scripts/config --enable NET_CLS_CGROUP
scripts/config --enable IP_NF_TARGET_REDIRECT
scripts/config --enable BRIDGE_VLAN_FILTERING

# Additional components for features like IPv6, IPVS, network namespaces
scripts/config --enable IP_VS
scripts/config --enable IP_VS_NFCT
scripts/config --enable IP_VS_PROTO_TCP
scripts/config --enable IP_VS_PROTO_UDP
scripts/config --enable IP_VS_RR
scripts/config --enable IPVLAN
scripts/config --enable MACVLAN
scripts/config --enable DUMMY

# Components for encrypted networks
scripts/config --enable XFRM
scripts/config --enable XFRM_USER
scripts/config --enable XFRM_ALGO
scripts/config --enable INET_ESP
scripts/config --enable NETFILTER_XT_MATCH_BPF

# FTP/TFTP support for containers
scripts/config --enable NF_NAT_FTP
scripts/config --enable NF_CONNTRACK_FTP
scripts/config --enable NF_NAT_TFTP
scripts/config --enable NF_CONNTRACK_TFTP

# Additional storage drivers if needed
scripts/config --module BTRFS_FS
scripts/config --module BTRFS_FS_POSIX_ACL

yes "" | make localmodconfig

# Keep host networking and config introspection support after localmodconfig.
scripts/config --enable IP6_NF_FILTER
scripts/config --enable BRIDGE_VLAN_FILTERING
scripts/config --enable IKCONFIG
scripts/config --enable IKCONFIG_PROC
yes "" | make olddefconfig

fakeroot make -j$(($(nproc) - 1))

sudo apt-get install -y \
    libdw-dev \
    systemtap-sdt-dev \
    libunwind-dev \
    libslang2-dev \
    libperl-dev \
    liblzma-dev \
    libcap-dev \
    libnuma-dev \
    libbabeltrace-dev \
    libpfm4-dev \
    libtraceevent-dev \
    libbfd-dev \
    libzstd-dev

cd tools/perf
make -j
sudo make install
sudo cp perf /usr/bin/perf
cd ../..

sudo make modules_install
sudo make install

sudo DEBIAN_FRONTEND=noninteractive apt-get install kexec-tools -y
# sudo kexec -l arch/x86/boot/bzImage --append="$(cat /proc/cmdline) modulepath=$(pwd)/lib/modules" --reuse-cmdline
# cat /sys/kernel/kexec_loaded
# sudo kexec -l /boot/vmlinuz-6.6.36 --initrd=/boot/initrd.img-6.6.36 --append="$(cat /proc/cmdline)"
# sudo kexec -e

# cd module
# make -j

# sudo insmod kutrace_mod.ko tracemb=40 check=0
# sudo rmmod kutrace_mod.ko

# cd ../

# cp postproc_changes/* postproc/

# cd postproc
# ./build.sh
