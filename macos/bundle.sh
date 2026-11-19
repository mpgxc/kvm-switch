#!/bin/sh
# Gera o instalador do app para macOS: target/release/KVM Switch.app e
# target/release/KVM-Switch-<versão>.dmg (arrastar o app para Aplicativos).
# Só usa ferramentas do próprio macOS além do cargo.
set -eu
cd "$(dirname "$0")/.."

[ "$(uname -s)" = Darwin ] || { echo "rode no macOS" >&2; exit 1; }

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
out=target/release
app="$out/KVM Switch.app"
dmg="$out/KVM-Switch-$version.dmg"

cargo build --release --locked

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$out/kvm-switch" "$app/Contents/MacOS/kvm-switch"

# Ícone do Dock: o mesmo 🖥 da barra de menus. O PNG tem 128 px, então os
# tamanhos maiores ficam de fora em vez de ampliados.
iconset=$(mktemp -d)/AppIcon.iconset
mkdir -p "$iconset"
for size in 16 32 64 128; do
  sips -z "$size" "$size" assets/icons/1f5a5.png --out "$iconset/tmp-$size.png" >/dev/null
done
mv "$iconset/tmp-16.png" "$iconset/icon_16x16.png"
cp "$iconset/tmp-32.png" "$iconset/icon_16x16@2x.png"
mv "$iconset/tmp-32.png" "$iconset/icon_32x32.png"
mv "$iconset/tmp-64.png" "$iconset/icon_32x32@2x.png"
mv "$iconset/tmp-128.png" "$iconset/icon_128x128.png"
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$iconset")"

cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>KVM Switch</string>
  <key>CFBundleDisplayName</key><string>KVM Switch</string>
  <key>CFBundleIdentifier</key><string>io.github.mpgxc.kvm-switch</string>
  <key>CFBundleExecutable</key><string>kvm-switch</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

# Assinatura ad hoc: basta para rodar nesta máquina. Em outro Mac o Gatekeeper
# barra o app baixado (veja o README); assinar e notarizar exige conta de
# desenvolvedor da Apple.
codesign --force --sign - "$app"

stage=$(mktemp -d)
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -volname "KVM Switch" -srcfolder "$stage" -format UDZO -ov "$dmg" >/dev/null
rm -rf "$stage"

echo "app: $app"
echo "instalador: $dmg"
