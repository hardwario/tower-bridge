# Linux notes

## Permissions

Install the udev rule so the logged-in user can open the bridge:

```sh
sudo cp packaging/linux/70-hardwario-tower-bridge.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

Re-plug the Bridge Module afterwards.

The rule gives access to the user logged in at the computer. A service or an
SSH session on a computer without a desktop (a Raspberry Pi running
`tower-bridge-cli run`) has no such login: add the user to the `plugdev` group,
which the rule also gives access, and log in again:

```sh
sudo usermod -aG plugdev $USER
```

Distributions without a `plugdev` group (Fedora) ignore that part of the rule;
create the group or change it in the rule.

## The kernel `hid-ft260` driver

Kernels 5.13 and newer contain a driver for the FT260. When it binds to the
bridge it registers an I2C adapter (`/dev/i2c-N`, named `FT260 …`) and newer
kernels no longer create a `/dev/hidraw*` node for it.

TOWER Bridge handles both cases:

1. If a hidraw node exists, the bridge is driven directly over USB HID (LED
   control included).
2. Otherwise it uses the kernel I2C adapter. This needs the `i2c-dev` module:

   ```sh
   sudo modprobe i2c-dev
   echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf
   ```

To always use direct HID access instead, blacklist the kernel driver:

```sh
echo "blacklist hid_ft260" | sudo tee /etc/modprobe.d/blacklist-hid-ft260.conf
```

## AppImage

The AppImage needs glibc 2.35 or newer and FUSE 2 (`libfuse2` on Ubuntu 22.04,
`libfuse2t64` on 24.04). Make it executable and run it:

```sh
chmod +x TOWER-Bridge-*-linux-x86_64.AppImage
./TOWER-Bridge-*-linux-x86_64.AppImage
```

"Open at login" registers the AppImage path itself, so keep the file where it
is (re-enable the option after moving it).

## Tray icon on GNOME

GNOME shows tray icons only with the *AppIndicator and KStatusNotifierItem
Support* extension (preinstalled on Ubuntu). KDE Plasma works out of the box.

## Rendering problems

On some Wayland/NVIDIA setups the dashboard window stays blank. Start the app
with `WEBKIT_DISABLE_DMABUF_RENDERER=1`.
