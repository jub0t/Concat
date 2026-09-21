# Maintainer: noxygalaxy <noxycraftyt@gmail.com>
pkgname=concat
pkgver=${CONCAT_VERSION:?CONCAT_VERSION must be set}
pkgrel=1
pkgdesc="Free and open source video editor"
arch=('x86_64' 'aarch64')
url="https://github.com/jub0t/Concat"
license=('AGPL-3.0-or-later')
depends=('glibc' 'gtk3' 'fontconfig' 'freetype2' 'libxkbcommon' 'mesa' 'alsa-lib')
options=('!strip' '!debug')
source=()

stage=${CONCAT_STAGE:?CONCAT_STAGE must point at the staged folder}

package() {
  install -Dm755 "$stage/concat" "$pkgdir/opt/concat/concat"
  install -d "$pkgdir/opt/concat/lib"
  cp -a "$stage"/lib/. "$pkgdir/opt/concat/lib/"

  install -d "$pkgdir/usr/bin"
  printf '#!/bin/sh\nexec /opt/concat/concat "$@"\n' > "$pkgdir/usr/bin/concat"
  chmod 755 "$pkgdir/usr/bin/concat"
  install -Dm644 "$stage/concat.desktop" "$pkgdir/usr/share/applications/concat.desktop"
  sed -i 's|^Exec=concat|Exec=/opt/concat/concat %U|' "$pkgdir/usr/share/applications/concat.desktop"
  install -Dm644 "$stage/concat.png" "$pkgdir/usr/share/icons/hicolor/256x256/apps/concat.png"

  install -Dm644 "$stage/LICENSE" "$pkgdir/usr/share/licenses/concat/LICENSE"
  install -Dm644 "$stage/THIRD_PARTY_NOTICES.md" "$pkgdir/usr/share/licenses/concat/THIRD_PARTY_NOTICES.md"
}
