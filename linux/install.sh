#!/usr/bin/env bash
# Instala o kvm-switch no Linux (Ubuntu/Debian com GNOME), com tudo que ele usa.
#
#   ./install.sh         versão em script: atalhos Super+Alt+1/2/3/0 já ativos
#   ./install.sh --app   app em Rust: compila, instala e abre; os atalhos são definidos nele
#
# Pode rodar de novo à vontade: só faz o que falta, e só pede a senha do sudo
# quando há pacote de sistema para instalar. Rode como seu usuário, sem sudo.
set -euo pipefail
cd "$(dirname "$0")"

case "${1:-}" in
  "") mode=scripts ;;
  --app) mode=app ;;
  *) echo "uso: $0 [--app]" >&2; exit 2 ;;
esac

command -v apt-get >/dev/null || {
  echo "requer apt (Ubuntu/Debian). Em outra distribuição instale à mão o ddcutil e o solaar;" \
    "para os scripts, também python3 e notify-send; para o app, Rust e as bibliotecas de" \
    "desenvolvimento do xkbcommon, xcb, Wayland e fontconfig." >&2
  exit 1
}

# ddcutil: fala DDC/CI com o monitor e libera /dev/i2c-* ao usuário logado.
# solaar: libera os receptores Logitech (regra udev) e serve de diagnóstico.
# libglib2.0-bin: gsettings, que cria os atalhos do GNOME.
packages=(ddcutil solaar libglib2.0-bin)
if [ "$mode" = scripts ]; then
  # python3 roda o logi-host; libnotify-bin tem o notify-send dos avisos.
  packages+=(python3 libnotify-bin)
else
  # O que o GPUI precisa para compilar e abrir janelas (Wayland e X11, por Vulkan).
  packages+=(build-essential pkg-config curl libxkbcommon-dev libxkbcommon-x11-dev
    libxcb1-dev libx11-xcb-dev libwayland-dev libfontconfig-dev libfreetype-dev libvulkan1)
fi
missing=()
for pkg in "${packages[@]}"; do
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

schema=org.gnome.settings-daemon.plugins.media-keys
base=/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings
gnome=false
gsettings list-schemas 2>/dev/null | grep -qx "$schema" && gnome=true

install_scripts() {
  # Os links apontam para esta pasta: editar aqui vale direto.
  mkdir -p "$HOME/.local/bin"
  local f dest bak
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

  if ! $gnome; then
    echo "AVISO: este ambiente gráfico não é GNOME. Crie os atalhos nele, apontando para:"
    echo "  $HOME/.local/bin/monitor-input usbc|dp|hdmi|next"
    return
  fi
  local list id key arg name path
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
}

install_app() {
  PATH="$HOME/.cargo/bin:$PATH"
  command -v cargo >/dev/null ||
    curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
  cargo install --path .. --locked --root "$HOME/.local"

  mkdir -p "$HOME/.local/share/applications"
  cat >"$HOME/.local/share/applications/kvm-switch.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=KVM Switch
Comment=Troca a entrada do monitor e o canal dos periféricos Logitech por atalho
Exec="$HOME/.local/bin/kvm-switch"
Icon=video-display
Terminal=false
Categories=Utility;
StartupWMClass=kvm-switch
EOF

  if $gnome; then
    # Os atalhos da versão em script ocupam as teclas: enquanto existirem, o GNOME
    # não as entrega ao app, nem para gravar um atalho novo.
    local list path kept=()
    list="$(gsettings get $schema custom-keybindings)"
    if [[ "$list" == *"$base/monitor-input-"* ]]; then
      while IFS= read -r path; do
        if [[ "$path" == "$base/monitor-input-"* ]]; then
          gsettings reset-recursively "$schema.custom-keybinding:$path"
        elif [ -n "$path" ]; then
          kept+=("'$path'")
        fi
      done < <(tr -d "[]' " <<<"$list" | tr ',' '\n')
      if ((${#kept[@]})); then
        gsettings set $schema custom-keybindings "[$(IFS=,; echo "${kept[*]}")]"
      else
        gsettings set $schema custom-keybindings "@as []"
      fi
      echo "os atalhos Super+Alt+1/2/3/0 da versão em script foram removidos"
    fi
  else
    echo "AVISO: este ambiente gráfico não é GNOME. O app não cria os atalhos sozinho:"
    echo "  crie-os no seu ambiente rodando $HOME/.local/bin/kvm-switch switch <entrada>"
  fi

  # Abre a janela de configuração (ou a traz de volta, se o app já estiver aberto).
  (setsid "$HOME/.local/bin/kvm-switch" >/dev/null 2>&1 &)
}

if [ "$mode" = scripts ]; then install_scripts; else install_app; fi

echo
ddcutil detect --brief 2>/dev/null | grep -q '^Display' ||
  echo "AVISO: nenhum monitor respondeu por DDC/CI. Confira o cabo e o DDC/CI no menu do monitor."
grep -qsi 'HID_ID=0003:0000046D:' /sys/class/hidraw/*/device/uevent ||
  echo "AVISO: receptor Logitech não encontrado nesta máquina."
if [ "$mode" = scripts ]; then
  echo "Instalado. Aperte Super+Alt+2: deve aparecer a notificação \"Já está em DisplayPort (este PC); nada a trocar.\""
else
  echo "Instalado em ~/.local/bin/kvm-switch. A janela \"KVM Switch\" abriu: defina nela os destinos e os atalhos."
fi
