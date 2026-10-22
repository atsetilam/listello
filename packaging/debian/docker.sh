#!/bin/sh
# Build the .deb inside a Debian 13 container, for a machine that is not Debian-based.
#
#   BUILDDIR=/some/disk/build PKGDEST=/some/disk/out packaging/debian/docker.sh
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
builddir=${BUILDDIR:-$root/../listello-deb-build}
pkgdest=${PKGDEST:-$builddir}
mkdir -p "$builddir" "$pkgdest"

# The tree is handed over as a tarball (a git worktree's .git points outside the checkout, so
# git cannot run in the container); the build happens in BUILDDIR.
(cd "$root" && git ls-files -z --cached --others --exclude-standard | tar --null -T - -cf "$builddir/source.tar")
docker run --rm \
  -v "$root:/src:ro" -v "$builddir:/build" -v "$pkgdest:/out" \
  -e SOURCE_TAR=/build/source.tar \
  -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
  debian:trixie sh -c '
    set -eu
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends \
      build-essential debhelper dpkg-dev ca-certificates fakeroot \
      rustup clang libclang-dev pkgconf qt6-base-dev qt6-declarative-dev \
      libcairo2-dev libdbus-1-dev libfontconfig-dev libfreetype-dev libinput-dev \
      libpipewire-0.3-dev librsvg2-dev libudev-dev
    rustup default stable >/dev/null 2>&1
    export PATH="$HOME/.cargo/bin:$PATH"
    BUILDDIR=/build PKGDEST=/out /src/packaging/debian/build.sh
    chown -R "$HOST_UID:$HOST_GID" /build /out
  '
