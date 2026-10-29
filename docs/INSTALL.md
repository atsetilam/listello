# Installing listello

listello is built and tested on **Arch Linux with KDE Plasma (Wayland)** on a MacBookPro16,1
(T2), using the [t2linux](https://wiki.t2linux.org/) kernel. That is the only combination
checked on real hardware. Other T2 Macs share the same panel code as
[tiny-dfr](https://github.com/AsahiLinux/tiny-dfr) and should work. Apple Silicon under Asahi
Linux builds, but nobody has tried it on a real machine yet.

Coming from tiny-dfr? Read [FROM-TINY-DFR.md](FROM-TINY-DFR.md). Coming from Pock on macOS?
Read [FROM-POCK.md](FROM-POCK.md).

## What you need first

- A kernel that drives the Touch Bar: on a T2 Mac that is `linux-t2` (the `appletbdrm` and
  `hid-appletb-bl` drivers). If the Touch Bar shows the function row under tiny-dfr, the
  kernel side is fine.
- KDE Plasma 6 on Wayland, for the Dock. Without it the strip still works — function keys,
  media keys, clock — and the Dock stays empty. Other desktops are not supported yet.

## Install

Installing listello **turns nothing on**, for anybody. Every route below ends at
[Turn it on](#turn-it-on).

### Arch Linux: from the AUR

```sh
git clone https://aur.archlinux.org/listello-git.git
cd listello-git
makepkg -si
```

This builds the latest commit. The package replaces `tiny-dfr` (pacman will offer to remove
it).

### Arch Linux: from a checkout

```sh
git clone https://github.com/atsetilam/listello
cd listello/packaging/arch
makepkg -si
```

`makepkg` builds under `src/` in that directory and puts the package next to it; to keep
the build off a small disk, set `BUILDDIR` and `PKGDEST` to somewhere with room (the build
needs a few GB).

### Debian: the `.deb`

The `.deb` is built from a checkout. Debian 13 (trixie) is the release it is built on.
Other Debian-based systems with Qt 6.8 or newer should work, but have not been tried.

```sh
git clone https://github.com/atsetilam/listello
cd listello
sudo apt install build-essential debhelper dpkg-dev fakeroot rustup clang libclang-dev \
  pkgconf qt6-base-dev qt6-declarative-dev libcairo2-dev libdbus-1-dev libfontconfig-dev \
  libfreetype-dev libinput-dev libpipewire-0.3-dev librsvg2-dev libudev-dev
rustup default stable
BUILDDIR=$HOME/listello-build PKGDEST=$HOME/listello-deb packaging/debian/build.sh
sudo apt install $HOME/listello-deb/listello_*.deb
```

The build needs Rust 1.92 or newer, which Debian 13's own packages are too old for, hence
`rustup`. It downloads the crates it needs, so it wants the network. The package replaces
`tiny-dfr` and pulls in the Qt and Kirigami modules the settings window uses.

On a machine that is not Debian-based, `packaging/debian/docker.sh` does the same inside a
Debian 13 container (it needs Docker, and takes the same `BUILDDIR` and `PKGDEST`).

### From source, on any distribution

You need Rust (1.92 or newer), a C++ compiler, clang, `pkg-config`, and the development files
for Qt 6 (Base and Declarative), cairo, D-Bus, fontconfig, freetype, libinput, PipeWire,
librsvg and libudev. On Apple Silicon (aarch64) you also need `lld`: the build links with it
there. At run time the settings window needs the Kirigami and
`qqc2-desktop-style` QML modules and the Inter font.

```sh
git clone https://github.com/atsetilam/listello
cd listello
make
sudo make install
```

`make install` copies the programs, the icons, the systemd units and the udev rules into
`/usr` (the units name `/usr/bin`, so listello is meant to live there). `DESTDIR=` puts
everything under another root instead, for building your own package. Remove tiny-dfr first:
the two must not both be installed.

After installing from source, tell udev and systemd about the new files:

```sh
sudo systemctl daemon-reload
sudo udevadm control --reload
```

## Turn it on

The system side is one socket, which the administrator enables once:

```sh
sudo systemctl enable --now listello-core.socket
```

Then each user opens **listello** from the application menu. That starts listello on the
Touch Bar and opens the settings window. The first time, it asks whether to start listello
every time you log in (**Yes, every time** or **Only now**); the choice can be changed later
with *Start at login* in the *General* tab.

On a T2 Mac the renderer runs only while somebody is running listello. *Stop listello*, in the
window or from the tray icon, stops it and gives the Touch Bar back to the keys the Mac draws
itself.

### Two things that are easy to get wrong

- **Enable the socket while `listello-core.service` is stopped.** A socket will not start while
  its service is running. To restart both:
  `sudo systemctl stop listello-core; sudo systemctl start listello-core.socket`.
- **The agent must be the installed one.** KDE only shows a program the list of open windows
  if a desktop file names it, and that desktop file names `/usr/bin/listello-agent`. Running
  the agent from a build directory shows an empty Dock.

## Configure

The settings window edits your own layout in `~/.config/listello/config.toml`. The machine-wide
default can be overridden by copying it and editing the copy; listello reloads it when it
changes:

```sh
sudo mkdir -p /etc/listello
sudo cp /usr/share/listello/config.toml /etc/listello/config.toml
```

`/usr/share/listello/config.example-dock.toml` is a fuller layout with a Dock, folders,
controls, wifi and battery. Icon names are the files in `/usr/share/listello/` with dashes
instead of underscores (`trash-full` is `trash_full.svg`).

## Going back

```sh
sudo systemctl disable --now listello-core.socket
sudo pacman -R listello        # or: sudo apt remove listello
sudo pacman -S tiny-dfr        # or: sudo apt install tiny-dfr
```

Installed from source with `make install`? Delete the files it put under `/usr` by hand;
`make install DESTDIR=/some/empty/dir` shows the full list.
