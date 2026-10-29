#!/bin/sh
# Build the .rpm inside a Fedora container (release 44 when this was written), for a machine
# that is not Fedora.
#
#   BUILDDIR=/some/disk/build PKGDEST=/some/disk/out packaging/fedora/docker.sh
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
builddir=${BUILDDIR:-$root/../listello-rpm-build}
pkgdest=${PKGDEST:-$builddir}
mkdir -p "$builddir" "$pkgdest"

# The tree is handed over as a tarball (a git worktree's .git points outside the checkout, so
# git cannot run in the container); the build happens in BUILDDIR.
(cd "$root" && git ls-files -z --cached --others --exclude-standard | tar --null -T - -cf "$builddir/source.tar")
docker run --rm \
  -v "$root:/src:ro" -v "$builddir:/build" -v "$pkgdest:/out" \
  -e SOURCE_TAR=/build/source.tar \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  fedora:latest sh -c '
    set -eu
    dnf install -y -q rpm-build make gcc-c++ cargo rust clang clang-devel lld pkgconf-pkg-config \
      qt6-qtbase-devel qt6-qtdeclarative-devel cairo-devel dbus-devel fontconfig-devel \
      freetype-devel libinput-devel pipewire-devel librsvg2-devel systemd-devel \
      systemd-rpm-macros
    BUILDDIR=/build PKGDEST=/out /src/packaging/fedora/build.sh
    chown -R "$HOST_UID:$HOST_GID" /build /out
  '
