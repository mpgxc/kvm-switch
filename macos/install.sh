#!/bin/sh
# Instala no macOS tudo que o kvm-switch usa e deixa os atalhos ⌘⌥1 / ⌘⌥2 ativos.
# Pode rodar de novo à vontade: só faz o que falta.
set -eu
cd "$(dirname "$0")"
PATH="/opt/homebrew/bin:$PATH"

[ "$(uname -sm)" = "Darwin arm64" ] || { echo "requer macOS em Apple Silicon (exigência do m1ddc)" >&2; exit 1; }

command -v brew >/dev/null ||
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

brew install m1ddc hidapitester
[ -d /Applications/Hammerspoon.app ] || brew install --cask hammerspoon

# O Hammerspoon lê ~/.hammerspoon. Os links apontam para esta pasta: editar aqui vale direto.
mkdir -p "$HOME/.hammerspoon"
for f in init.lua kvm-switch.sh; do
  dest="$HOME/.hammerspoon/$f"
  # Arquivo que não é o nosso link vira backup em vez de ser sobrescrito.
  if [ -e "$dest" ] && [ "$(readlink "$dest" 2>/dev/null)" != "$PWD/$f" ]; then
    bak="$dest.bak-$(date +%Y%m%d%H%M%S)"
    mv "$dest" "$bak"
    echo "já existia $dest: guardado em $bak"
  fi
  ln -sf "$PWD/$f" "$dest"
done

# Reinicia para carregar o init.lua.
pkill -x Hammerspoon || true
sleep 1
open -a Hammerspoon

echo
# "display list" também lista a tela interna do MacBook; "get input" só passa com monitor externo.
m1ddc get input >/dev/null 2>&1 ||
  echo "AVISO: nenhum monitor externo respondeu por DDC. Confira o cabo USB-C e o DDC/CI no menu do monitor."
hidapitester --vidpid 046D:C548 --list 2>/dev/null | grep -q . ||
  echo "AVISO: receptor Logi Bolt não encontrado nesta máquina."
echo "Instalado. Aperte ⌘⌥1: deve aparecer a notificação \"você já está no Mac (USB-C)\"."
echo "Na primeira vez o macOS pergunta se permite notificações do Hammerspoon: permita."
echo "Se o atalho não disparar, libere o Hammerspoon em Ajustes → Privacidade e Segurança → Acessibilidade."
