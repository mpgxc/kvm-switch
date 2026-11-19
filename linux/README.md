# kvm-switch no Linux

Esta pasta tem o instalador do Linux e a versão em script: atalhos do GNOME que trocam a entrada do monitor por DDC/CI e, na ida para o Mac, levam junto o teclado e o mouse Logitech.

O app em Rust da raiz do repositório faz o mesmo com janela de configuração e destinos editáveis. Use um ou outro:

```sh
linux/install.sh         # versão em script, descrita abaixo
linux/install.sh --app   # app: compila, instala e abre (veja o README da raiz)
```

Os dois modos instalam o `ddcutil` e o `solaar`. O `--app` remove os atalhos da versão em script, porque duas coisas não podem ter a mesma tecla no GNOME.

## Versão em script

| Atalho | Ação |
|---|---|
| Super+Alt+1 | Mac: monitor em USB-C, teclado e mouse no canal 1 |
| Super+Alt+2 | este PC: monitor em DisplayPort |
| Super+Alt+3 | monitor em HDMI |
| Super+Alt+0 | próxima entrada (DP → HDMI → USB-C → DP) |

Se o monitor já está na entrada pedida, nada é enviado e aparece a notificação "Já está em …; nada a trocar.".

A volta do Mac não sai daqui: com teclado e mouse no Mac, este PC não recebe o atalho. Use o atalho do lado do Mac ([`../macos`](../macos/README.md)) ou o botão Easy-Switch 2 e depois Super+Alt+2.

## Instalação da versão em script

Ubuntu ou Debian com GNOME. Rode como seu usuário, sem `sudo` na frente:

```sh
git clone https://github.com/mpgxc/kvm-switch.git
kvm-switch/linux/install.sh
```

O `install.sh` pode ser rodado de novo sem risco e só pede a senha quando falta algo de sistema:

1. Instala pelo `apt` o que faltar:

   | Pacote | Para quê |
   |---|---|
   | `ddcutil` | Falar DDC/CI com o monitor. O pacote também libera `/dev/i2c-*` para o usuário logado |
   | `solaar` | Libera os receptores Logitech para o usuário logado (regra udev) e serve de diagnóstico (`solaar show`) |
   | `python3` | Roda o `logi-host` |
   | `libnotify-bin` | `notify-send`, para os avisos |
   | `libglib2.0-bin` | `gsettings`, para criar os atalhos |

2. Cria em `~/.local/bin` os links para `monitor-input` e `logi-host` desta pasta. Um arquivo diferente com esse nome é renomeado para `<nome>.bak-<data>` antes.
3. Cria os quatro atalhos personalizados do GNOME (Configurações → Teclado → Atalhos personalizados).
4. Confere se o monitor e o receptor respondem e avisa se não.

A pasta clonada precisa continuar no lugar, porque os links apontam para ela.

## Arquivos

| Arquivo | Função |
|---|---|
| `install.sh` | Instalador dos dois modos: dependências, links e atalhos, ou a compilação do app |
| `monitor-input` | Troca a entrada do monitor; para o Mac, chama o `logi-host` em paralelo |
| `logi-host` | Troca o canal Easy-Switch de todos os periféricos do receptor Logitech |
| `70-logi-bolt.rules` | Para quem não quer o Solaar: regra udev que libera só o receptor Bolt (`046d:c548`). Copie para `/etc/udev/rules.d/` e rode `sudo udevadm control --reload && sudo udevadm trigger` |

## Como funciona

### `monitor-input usbc|dp|hdmi|next|get`

- A entrada é o código VCP `0x60` do DDC/CI, enviado pelo `ddcutil`. Valores do Dell P3225QE: `0x1b` USB-C, `0x0f` DisplayPort, `0x11` HDMI.
- O barramento I2C do monitor fica guardado em `~/.cache/monitor-input-bus`. Sem isso o `ddcutil` procura o monitor a cada chamada (cerca de 1,5 s); com isso, ler ou trocar a entrada leva cerca de 0,2 s. Se o barramento guardado parar de responder, o script procura de novo.
- `get` imprime a entrada atual (`x0f`, `x11` ou `x1b`).
- Variáveis: `MONITOR_MODEL` (padrão `DELL P3225QE`) e `OTHER_PC_CHANNEL` (padrão `1`).

### `logi-host 1|2|3`

- Fala HID++ 2.0 direto com o `hidraw` do receptor: em cada um dos 6 slots pergunta o índice da feature `0x1814` (Change Host) e envia `setCurrentHost` a quem responder.
- Leva cerca de 0,07 s com os periféricos acordados e até 1 s se algum estiver em repouso. O `solaar config … change-host` faz o mesmo em cerca de 2,7 s por dispositivo, por isso o Solaar fica só com a permissão e o diagnóstico.

## Testar pelo terminal

```sh
monitor-input get    # entrada atual, sem trocar nada
monitor-input dp     # neste PC: avisa que já está no DisplayPort
logi-host 2          # manda teclado e mouse para o canal em que já estão
ddcutil detect       # o monitor responde por DDC/CI?
solaar show          # periféricos pareados e canais
```

## Adaptar

- **Outro monitor:** veja o modelo com `ddcutil detect` e os valores da feature 60 com `ddcutil capabilities`; ajuste `MONITOR_MODEL` e os códigos no `monitor-input` e apague `~/.cache/monitor-input-bus`.
- **Outro canal para o Mac:** `OTHER_PC_CHANNEL` no `monitor-input`.
- **Outros atalhos:** a tabela no fim do `install.sh`; rode-o de novo.

## Problemas comuns

| Sintoma | O que ver |
|---|---|
| "Monitor … não encontrado via DDC/CI" | DDC/CI ligado no menu do monitor; `ddcutil detect`. Logo após instalar o `ddcutil`, saia e entre na sessão se `/dev/i2c-*` continuar sem acesso |
| `logi-host` responde "nenhum dispositivo trocado" | O receptor está nesta máquina? `ls -l /dev/hidraw*` deve mostrar `+` (acesso do usuário) nos do receptor |
| A tela foi para outra entrada sozinha | Sem sinal na entrada escolhida o monitor procura outra. Ligue a outra máquina ou desative a seleção automática no menu do monitor |
| Teclado ficou preso na outra máquina | Botão Easy-Switch do teclado e do mouse |
