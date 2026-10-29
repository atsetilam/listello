#!/bin/sh
# Build the .rpm on a Fedora machine that has the packages in listello.spec's BuildRequires
# installed (and the network, to fetch the crates).
#
#   BUILDDIR=/some/disk/build PKGDEST=/some/disk/out packaging/fedora/build.sh
#
# SOURCE_TAR names a ready-made tarball of the tree (docker.sh uses it). Builds from the working tree, not from a commit, in a copy under BUILDDIR.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
builddir=${BUILDDIR:-$root/../listello-rpm-build}
pkgdest=${PKGDEST:-$builddir}
spec=$root/packaging/fedora/listello.spec
version=$(sed -n 's/^Version:[[:space:]]*//p' "$spec")
top=$builddir/rpmbuild

rm -rf "$top"
mkdir -p "$top/SOURCES" "$pkgdest"
if [ -n "${SOURCE_TAR:-}" ]; then
  cp "$SOURCE_TAR" "$top/SOURCES/listello-$version.tar"
else
  (cd "$root" && git ls-files -z --cached --others --exclude-standard | tar --null -T - -cf -) > "$top/SOURCES/listello-$version.tar"
fi

rpmbuild -bb --define "_topdir $top" "$spec"
mv "$top"/RPMS/*/listello-*.rpm "$pkgdest"/
