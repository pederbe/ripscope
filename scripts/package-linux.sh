#!/usr/bin/env bash
# Package an inspected archive directory without rebuilding or rewriting its binary.
# Usage: bash scripts/package-linux.sh VERSION PACKAGE_ROOT OUT_DIR
set -euo pipefail
version=${1:?version required}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Use a numeric release version' >&2; exit 1; }
package=$(realpath "${2:?archive directory required}")
mkdir -p "${3:?output directory required}"
out=$(realpath "$3")
for tool in dpkg-deb rpmbuild rpm2cpio cpio sha256sum; do
    command -v "$tool" >/dev/null || { echo "Required package tool missing: $tool" >&2; exit 1; }
done
(cd "$package" && sha256sum -c MANIFEST.sha256)
test -s "$package/contrib/emacs/vendor/COPYING"
test -s "$package/contrib/emacs/vendor/xcscope.el"
export SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(stat -c %Y "$package/bin/ripscope")}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
same() {
    cmp "$package/bin/ripscope" "$1/usr/bin/ripscope"
    # The complete archive support material is distributed, including xcscope source.
    diff -r "$work/support" "$1/usr/share/ripscope"
}
mkdir "$work/support"
cp -a "$package/." "$work/support/"
rm -rf "$work/support/bin"
mv "$work/support/MANIFEST.sha256" "$work/support/ORIGINAL-ARCHIVE-MANIFEST.sha256"
binary_hash=$(sha256sum "$package/bin/ripscope"); binary_hash=${binary_hash%% *}
printf 'Original archive manifest: bin/ripscope installed at /usr/bin/ripscope\nExecutable SHA-256: %s\n' "$binary_hash" > "$work/support/INSTALLATION.txt"
mkdir -p "$work/deb/usr/bin" "$work/deb/usr/share/ripscope" "$work/deb/DEBIAN"
cp -a "$work/support/." "$work/deb/usr/share/ripscope/"
install -m755 "$package/bin/ripscope" "$work/deb/usr/bin/ripscope"
cat > "$work/deb/DEBIAN/control" <<CONTROL
Package: ripscope
Version: $version
Architecture: amd64
Maintainer: Peder Bergan <https://github.com/pederbe/ripscope/issues>
Depends: libc6 (>= 2.28), libgcc-s1
Section: devel
Priority: optional
Homepage: https://ripscope.com
Description: C navigation with atomic index updates
 Cscope searches for C, with stored queries and live text searches.
Installed-Size: $(du -sk --exclude=DEBIAN "$work/deb" | cut -f1)
CONTROL
same "$work/deb"
deb="ripscope_${version}_amd64.deb"
dpkg-deb --root-owner-group -Zxz --build "$work/deb" "$out/$deb" >/dev/null
dpkg-deb -x "$out/$deb" "$work/deb-check"
same "$work/deb-check"
top=$work/rpm
mkdir -p "$top"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
# A fixed staging path avoids interpolating caller-controlled paths into RPM code.
cp -a "$work/deb/usr" "$top/SOURCES/usr"
cat > "$top/SPECS/ripscope.spec" <<'SPEC'
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none
Name: ripscope
Version: %{release_version}
Release: 1
Summary: C navigation with atomic index updates
License: (MIT OR Apache-2.0) AND GPL-2.0-or-later
URL: https://ripscope.com
BuildArch: x86_64
AutoReqProv: no
Requires: glibc >= 2.28
Requires: libgcc
%description
Cscope searches for C, with stored queries and live text searches.
The optional Emacs integration includes separately licensed xcscope source.
%install
mkdir -p %{buildroot}/usr
cp -a %{_sourcedir}/usr/. %{buildroot}/usr/
%files
/usr/bin/ripscope
/usr/share/ripscope
SPEC
rpmbuild --quiet -bb --define "_topdir $top" --define "release_version $version" \
    --define 'use_source_date_epoch_as_buildtime 1' --define 'clamp_mtime_to_source_date_epoch 1' \
    "$top/SPECS/ripscope.spec"
rpm="ripscope-$version-1.x86_64.rpm"
mkdir "$work/rpm-check"
(cd "$work/rpm-check" && rpm2cpio "$top/RPMS/x86_64/$rpm" | cpio -idm --quiet)
same "$work/rpm-check"
cp "$top/RPMS/x86_64/$rpm" "$out/$rpm"
(cd "$out" && sha256sum "$deb" "$rpm" > PACKAGES.sha256 &&
    sha256sum "$deb" > "$deb.sha256" && sha256sum "$rpm" > "$rpm.sha256")
printf '%s\n' "$out/$deb" "$out/$rpm"
