#!/bin/sh
# Gera o pacote do app para Ubuntu/Debian: target/release/kvm-switch_<versão>_<arq>.deb.
# Instala o mesmo que o `install.sh --app` (binário, atalho no menu, ícone na barra a
# cada login), só que para todos os usuários; o apt traz as dependências de sistema.
# Requer o cargo e as bibliotecas de desenvolvimento listadas no install.sh.
set -eu
cd "$(dirname "$0")/.."

command -v dpkg-shlibdeps >/dev/null || { echo "requer dpkg-dev (Ubuntu/Debian)" >&2; exit 1; }

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
arch=$(dpkg --print-architecture)
out=target/release
deb="$out/kvm-switch_${version}_$arch.deb"

cargo build --release --locked

root=$(mktemp -d)
# O mktemp cria a pasta com 0700, e ela vira a "./" do pacote.
chmod 755 "$root"
install -Dm755 -s "$out/kvm-switch" "$root/usr/bin/kvm-switch"

mkdir -p "$root/usr/share/applications" "$root/etc/xdg/autostart"
cat >"$root/usr/share/applications/kvm-switch.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=KVM Switch
Comment=Troca a entrada do monitor e o canal dos periféricos Logitech por atalho
Exec=/usr/bin/kvm-switch
Icon=video-display
Terminal=false
Categories=Utility;
StartupWMClass=kvm-switch
EOF
cat >"$root/etc/xdg/autostart/kvm-switch-tray.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=KVM Switch (ícone na barra)
Comment=Ícone do KVM Switch na barra superior
Exec=/usr/bin/kvm-switch tray
Icon=video-display
Terminal=false
NoDisplay=true
X-GNOME-Autostart-enabled=true
EOF

# Bordas: a extensão do GNOME Shell (45 ou mais novo), para todos os usuários. O app a
# liga quando as Bordas são ligadas; no Wayland ela só carrega numa sessão nova.
ext=kvm-switch@mpgxc.github.io
for f in "linux/gnome-extension/$ext"/*; do
  install -Dm644 "$f" "$root/usr/share/gnome-shell/extensions/$ext/${f##*/}"
done

# Bibliotecas ligadas no binário; o dpkg-shlibdeps só roda ao lado de um debian/control.
mkdir "$root/debian"
echo 'Source: kvm-switch' >"$root/debian/control"
libs=$(cd "$root" && dpkg-shlibdeps -O usr/bin/kvm-switch | sed -n 's/^shlibs:Depends=//p')
rm -r "$root/debian"

# libvulkan1: o GPUI a abre com dlopen, então o dpkg-shlibdeps não a vê.
# ddcutil fala com o monitor; libglib2.0-bin traz o gsettings dos atalhos do GNOME;
# solaar libera os receptores Logitech (regra udev).
mkdir "$root/DEBIAN"
cat >"$root/DEBIAN/control" <<EOF
Package: kvm-switch
Version: $version
Architecture: $arch
Maintainer: mpgxc <mpgx5.c@gmail.com>
Section: utils
Priority: optional
Homepage: https://github.com/mpgxc/kvm-switch
Depends: $libs, libvulkan1, ddcutil, libglib2.0-bin
Recommends: solaar
Description: Troca a entrada do monitor e o canal dos periféricos Logitech por atalho
 Muda junto a entrada do monitor (DDC/CI) e o canal Easy-Switch do teclado e
 do mouse Logitech num receptor Bolt.
EOF

rm -f "$deb"
dpkg-deb --root-owner-group --build "$root" "$deb" >/dev/null
rm -rf "$root"

echo "pacote: $deb"
