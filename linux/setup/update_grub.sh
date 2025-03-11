#!/bin/bash

# Define the kernel version
KERNEL_VERSION="6.6.36"

# Check if running as root
if [ "$(id -u)" -ne 0 ]; then
    echo "This script must be run as root. Try using sudo."
    exit 1
fi

echo "Setting up to boot into kernel version $KERNEL_VERSION..."

# Modify GRUB default
if [ -f /etc/default/grub ]; then
    # Backup the original file
    cp /etc/default/grub /etc/default/grub.bak
    
    # Update the default kernel
    sed -i "s/^GRUB_DEFAULT=.*/GRUB_DEFAULT=\"Advanced options for $(grep -o "^ID=.*" /etc/os-release | cut -d= -f2 | tr -d '"')\>$(grep -o "^ID=.*" /etc/os-release | cut -d= -f2 | tr -d '"'), with Linux $KERNEL_VERSION\"/" /etc/default/grub
    
    # Update GRUB
    echo "Updating GRUB configuration..."
    update-grub || grub2-mkconfig -o /boot/grub/grub.cfg || grub2-mkconfig -o /boot/grub2/grub.cfg
else
    echo "Error: Could not find GRUB configuration file."
    exit 1
fi

echo "GRUB updated. You can now reboot..."
# reboot