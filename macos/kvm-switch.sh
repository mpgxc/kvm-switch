#!/bin/sh
# kvm-switch.sh 1|2: troca a entrada do Dell P3225QE e o canal Easy-Switch dos
# periféricos Logitech pareados no receptor Bolt DESTA máquina.
#   1 = USB-C + canal 1 (Mac)    2 = DP + canal 2 (Linux)
# Mesmo arquivo nas duas máquinas: só quem está com teclado/mouse consegue mandá-los embora.
set -eu
PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"

case "${1:-}" in
  1) input=USB-C vcp=0x1b host=0 name=Mac ;;
  2) input=DP    vcp=0x0f host=1 name=Linux ;;
  *) echo "uso: $0 1|2" >&2; exit 2 ;;
esac

# Monitor em paralelo com a varredura dos slots.
monitor() {
  if [ "$(uname)" = Darwin ]; then
    # DDC direto: o CLI do DDPM leva ~6 s por troca, o m1ddc ~0,1 s.
    m1ddc set input "$((vcp))" >/dev/null
  else
    ddcutil --model "DELL P3225QE" setvcp 60 "$vcp"
  fi
}

# O atalho só chega em quem está com o teclado: destino = esta máquina é "já estou aqui".
self=2; [ "$(uname)" = Darwin ] && self=1
if [ "$1" = "$self" ]; then
  monitor >/dev/null 2>&1 & # traz a tela de volta caso o monitor tenha ido para outra entrada
  msg="você já está no $name ($input)"
  echo "$msg" >&2 # no Mac o Hammerspoon mostra o stderr como notificação
  command -v notify-send >/dev/null && notify-send KVM "$msg"
  exit 3
fi

monitor &
mon=$!

# HID++ 2.0 pelo receptor Bolt (046D:C548): relatório longo 0x11, software id 0xe.
hidpp() {
  msg=$1; shift
  hidapitester --vidpid 046D:C548 --usagePage 0xFF00 --usage 2 --open --length 20 \
    --send-output "0x11,$msg" "$@"
}

targets=
for slot in 1 2 3 4 5 6; do
  # IRoot.getFeature(0x1814 CHANGE_HOST): o índice da feature varia por dispositivo.
  # Só vale linha depois de "Reading": o hidapitester também ecoa o pacote enviado.
  # 1ª leitura espera o dispositivo acordar; a 2ª cobre resposta atrasada de outro slot na frente.
  idx=$(hidpp "$slot,0x00,0x0e,0x18,0x14" --timeout 1000 --read-input --timeout 100 --read-input |
    awk -v s="0$slot" '/Reading/ { r = 1 } r && $1 == "11" && $2 == s && $3 == "00" && $4 == "0E" { print $5; exit }')
  [ -n "$idx" ] && [ "$idx" != 00 ] || continue
  targets="$targets $slot,0x$idx"
done

# Periféricos só saem se o monitor trocou: senão teclado e mouse continuam aqui.
wait "$mon" || { echo "monitor não trocou para $input" >&2; exit 1; }
[ -n "$targets" ] || { echo "monitor em $input, mas nenhum periférico Logitech neste receptor" >&2; exit 1; }
for t in $targets; do hidpp "$t,0x1e,$host" >/dev/null; done # setCurrentHost(host)
