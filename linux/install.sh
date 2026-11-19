#!/usr/bin/env bash
# Instala no Linux (Ubuntu/Debian com GNOME) tudo que o kvm-switch usa e deixa os
# atalhos Super+Alt+1/2/3/0 ativos. Pode rodar de novo à vontade: só faz o que falta,
# e só pede a senha do sudo quando há algo de sistema para instalar.
set -euo pipefail
cd "$(dirname "$0")"

command -v apt-get >/dev/null || {
  echo "requer apt (Ubuntu/Debian). Em outra distribuição instale à mão:" \
    "ddcutil, solaar, python3 e notify-send (libnotify)." >&2
  exit 1
}

# ddcutil: fala DDC/CI com o monitor e libera /dev/i2c-* ao usuário logado.
# solaar: libera os receptores Logitech (regra udev) e serve de diagnóstico.
# libnotify-bin: notify-send. libglib2.0-bin: gsettings.
missing=()
for pkg in ddcutil solaar python3 libnotify-bin libglib2.0-bin; do
  dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q 'ok installed' || missing+=("$pkg")
done
if ((${#missing[@]})); then
  sudo apt-get update
  sudo apt-get install -y "${missing[@]}"
  # As regras udev dos pacotes só valem para o que já está conectado depois de reaplicadas.
  sudo udevadm control --reload
  sudo udevadm trigger --subsystem-match=hidraw --subsystem-match=i2c-dev
fi
# O ddcutil usa /dev/i2c-*; o pacote só carrega o módulo no próximo boot.
compgen -G '/dev/i2c-*' >/dev/null || sudo modprobe i2c-dev

# Os links apontam para esta pasta: editar aqui vale direto.
mkdir -p "$HOME/.local/bin"
for f in monitor-input logi-host; do
  dest="$HOME/.local/bin/$f"
  # Arquivo diferente que não é o nosso link vira backup em vez de ser sobrescrito.
  if [ -e "$dest" ] && [ "$(readlink "$dest" 2>/dev/null)" != "$PWD/$f" ] && ! cmp -s "$f" "$dest"; then
    bak="$dest.bak-$(date +%Y%m%d%H%M%S)"
    mv "$dest" "$bak"
    echo "já existia $dest: guardado em $bak"
  fi
  ln -sf "$PWD/$f" "$dest"
done

schema=org.gnome.settings-daemon.plugins.media-keys
if gsettings list-schemas 2>/dev/null | grep -qx "$schema"; then
  base=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings
  list="$(gsettings get $schema custom-keybindings)"
  [ "$list" = "@as []" ] && list="[]"
  while read -r id key arg name; do
    path="$base/monitor-input-$id/"
    gsettings set "$schema.custom-keybinding:$path" name "KVM: $name"
    gsettings set "$schema.custom-keybinding:$path" command "$HOME/.local/bin/monitor-input $arg"
    gsettings set "$schema.custom-keybinding:$path" binding "$key"
    if [[ "$list" != *"'$path'"* ]]; then
      if [ "$list" = "[]" ]; then list="['$path']"; else list="${list%]}, '$path']"; fi
    fi
  done <<'EOF'
usbc <Super><Alt>1 usbc Mac (USB-C + teclado e mouse no canal 1)
dp   <Super><Alt>2 dp   este PC (DisplayPort)
hdmi <Super><Alt>3 hdmi HDMI
next <Super><Alt>0 next próxima entrada
EOF
  gsettings set $schema custom-keybindings "$list"
else
  echo "AVISO: este ambiente gráfico não é GNOME. Crie os atalhos nele, apontando para:"
  echo "  $HOME/.local/bin/monitor-input usbc|dp|hdmi|next"
fi

echo
ddcutil detect --brief 2>/dev/null | grep -q '^Display' ||
  echo "AVISO: nenhum monitor respondeu por DDC/CI. Confira o cabo e o DDC/CI no menu do monitor."
grep -qsi 'HID_ID=0003:0000046D:' /sys/class/hidraw/*/device/uevent ||
  echo "AVISO: receptor Logitech não encontrado nesta máquina."
echo "Instalado. Aperte Super+Alt+2: deve aparecer a notificação \"Já está em DisplayPort (este PC); nada a trocar.\""
