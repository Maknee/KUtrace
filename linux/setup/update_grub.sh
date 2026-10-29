#!/bin/bash

# Define the kernel version
KERNEL_VERSION="7.2.13"

# Check if running as root
if [ "$(id -u)" -ne 0 ]; then
    echo "This script must be run as root. Try using sudo."
    exit 1
fi

echo "Setting up to boot into kernel version $KERNEL_VERSION..."

if [ ! -f /etc/default/grub ]; then
    echo "Error: Could not find GRUB configuration file."
    exit 1
fi

# Regenerate grub.cfg first so the new kernel's entry exists, then take the menu
# titles from it. GRUB titles are case-sensitive ("Ubuntu", not os-release's "ubuntu");
# a GRUB_DEFAULT that matches nothing silently boots entry 0.
update-grub || grub2-mkconfig -o /boot/grub/grub.cfg || grub2-mkconfig -o /boot/grub2/grub.cfg
CFG=/boot/grub/grub.cfg
[ -f "$CFG" ] || CFG=/boot/grub2/grub.cfg
SUBMENU=$(grep -oP "^submenu '\K[^']+" "$CFG" | head -1)
ENTRY=$(grep -oP "menuentry '\K[^']*with Linux ${KERNEL_VERSION}(?=')" "$CFG" | head -1)
if [ -z "$SUBMENU" ] || [ -z "$ENTRY" ]; then
    echo "Error: no '... with Linux $KERNEL_VERSION' entry in $CFG. Install the kernel first."
    exit 1
fi

# Backup the original file, then update the default kernel
cp /etc/default/grub /etc/default/grub.bak
sed -i "s/^GRUB_DEFAULT=.*/GRUB_DEFAULT=\"$SUBMENU>$ENTRY\"/" /etc/default/grub
grep '^GRUB_DEFAULT=' /etc/default/grub

# Update GRUB
echo "Updating GRUB configuration..."
update-grub || grub2-mkconfig -o /boot/grub/grub.cfg || grub2-mkconfig -o /boot/grub2/grub.cfg

echo "GRUB updated. You can now reboot..."
# reboot
