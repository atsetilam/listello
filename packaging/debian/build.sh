#!/bin/sh
# Build the .deb on a Debian or Ubuntu machine that has the packages in debian/control's
# Build-Depends installed (and the network, to fetch the crates).
#
#   BUILDDIR=/some/disk/build PKGDEST=/some/disk/out packaging/debian/build.sh
#
# SOURCE_TAR names a ready-made tarball of the tree (docker.sh uses it). Builds from the working tree, not from a commit, in a copy under BUILDDIR.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
builddir=${BUILDDIR:-$root/../listello-deb-build}
pkgdest=${PKGDEST:-$builddir}
version=$(sed -n '1s/^listello (\(.*\)) .*/\1/p' "$root/packaging/debian/changelog")
tree=$builddir/listello-$version

rm -rf "$tree"
mkdir -p "$tree" "$pkgdest"
if [ -n "${SOURCE_TAR:-}" ]; then
  tar -C "$tree" -xf "$SOURCE_TAR"
else
  (cd "$root" && git ls-files -z --cached --others --exclude-standard | tar --null -T - -cf -) | tar -C "$tree" -xf -
fi
cp -r "$root/packaging/debian" "$tree/debian"
rm "$tree/debian/build.sh" "$tree/debian/docker.sh"

cd "$tree"
dpkg-buildpackage -b -us -uc
mv "$builddir"/listello_"$version"_*.deb "$pkgdest"/
