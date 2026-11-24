# Troca pela borda: teclado e mouse passam para o PC do lado

## Intenção

Hoje o app troca, por atalho, a entrada do monitor Dell e o canal Logitech juntos.
Falta o caso dos PCs lado a lado, cada um com a própria tela: levar o cursor até a
borda da tela e, como no Logitech Flow, teclado e mouse passarem para o PC vizinho.
Sem rede: cada PC roda o app e manda os periféricos embora quando o cursor encosta
na borda dele. Quem traz de volta é o app do outro PC, do mesmo jeito.

## Decisões do usuário

- Todo PC envolvido roda o kvm-switch. Não há distinção de sistema no modelo: é "um PC".
- **A borda nunca troca a entrada de vídeo.** Só teclado e mouse (HID++ `0x1814`
  `setCurrentHost`). Isso é invariante, com teste no domínio.
- Os modos são **dois interruptores independentes** na configuração, ligáveis ao mesmo tempo:
  - **Monitor compartilhado**: o que existe hoje (atalhos por destino, seletor, "Ir",
    itens "Ir para …" do ícone na barra, `kvm-switch switch N`). Troca vídeo + periféricos.
  - **Bordas**: a troca pela borda, só periféricos.
- O vizinho de uma borda é **só um canal Logitech (1–3) com um nome opcional**, sem
  ligação com a lista de destinos (um PC com monitor próprio não ocupa entrada do Dell).
- **Quatro lados**: esquerda, direita, cima, baixo.

## Objetivos

1. Configurar, em cada PC, o vizinho de cada lado (canal + nome opcional) num quadro
   3×3 na janela de configuração, com "Este PC" no centro.
2. Com o modo Bordas ligado, cursor na borda externa da área de trabalho, num lado
   com vizinho → periféricos vão para o canal daquele vizinho.
3. Os dois modos ligam e desligam na janela, cada um sozinho.
4. macOS e Linux (GNOME/Wayland).

## Não objetivos

- Passar a altura/posição do cursor para o outro PC, área de transferência, arrastar
  arquivos: exigiriam rede.
- Adapter de Windows.
- Atalho ou item no ícone da barra para ligar/desligar as bordas.
- Tecla modificadora obrigatória para atravessar ("segure Ctrl").

## Anti-objetivos (falha mesmo funcionando)

- A borda trocar a entrada do monitor, em qualquer caminho.
- Vai-e-volta: o mouse chega ao outro PC com o cursor já parado na borda de volta e
  retorna na hora.
- Uma atualização mudar o comportamento de quem já usa o app: `config.json` antigo
  carrega com Monitor compartilhado **ligado** e Bordas **desligado**.
- Troca pela borda no macOS levando ~2 s como a troca de periféricos de hoje.

## Restrições do projeto (de `AGENTS.md`)

- Identificadores, comentários, textos e erros em português; nomes de teste em inglês.
- Hexagonal: regras em `domain.rs`, contratos em `ports.rs`, sistema em `adapters/`.
- Campos novos de `Config` com `#[serde(default)]` e teste de arquivo antigo.
- `request.rs`: `arguments` e `parse` simétricos.
- Lints do `Cargo.toml`; `cargo test` e `cargo clippy --all-targets -- -D warnings`
  no sistema alterado. `README.md` (tabela de arquivos, uso, limites) e `AGENTS.md`
  atualizados.

## Desenho (escolhas minhas, ajustáveis)

### Configuração

```jsonc
{
  "targets": [...], "local_input": 27, "switcher_hotkey": "cmd+alt+space",
  "monitor_mode": true,              // novo; ausente => true
  "edges": {                         // novo; ausente => desligado e vazio
    "enabled": false,
    "left":   { "channel": 2, "name": "Linux" },
    "right":  null, "top": null, "bottom": null
  }
}
```

- Domínio: `enum Side { Left, Right, Top, Bottom }`, `struct Neighbor { channel: Channel, name: Option<String> }`,
  `struct Edges { enabled, left, right, top, bottom }` com `neighbor(side) -> Option<&Neighbor>`.
- `monitor_mode` usa `#[serde(default = ...)]` que devolve `true`.

### Monitor compartilhado desligado

- Atalhos dos destinos e do seletor não são registrados (`rebind` com lista vazia).
- Seletor não abre; ícone na barra sem "Ir para …"; botões "Ir" desabilitados.
- `kvm-switch switch N` não troca nada e mostra o aviso "O modo Monitor compartilhado
  está desligado", como os outros erros de troca (o pedido pode chegar pelo socket, que
  não devolve código de saída).
- A lista de destinos continua visível e editável (só apagada), como um destino inativo.

### Caso de uso da borda

- `Kvm::cross(side, &Edges) -> Result<Outcome>` (ou função equivalente) chama **só**
  `Peripherals::switch_to(neighbor.channel)`. Sem vizinho ou com Bordas desligado: não faz nada.
- Teste com o `Fake` existente: travessia nunca chama `Monitor::set_input` nem `current_input`.
- Uma travessia por vez: enquanto uma troca de periféricos roda, novos toques na borda
  são ignorados. Depois de uma troca, 1 s de espera antes de aceitar outra.
- Cantos não disparam (evita brigar com cantos ativos do macOS/GNOME).
- Sem aviso (HUD) em travessia com sucesso; erro aparece como aviso.

### Pedido `edge`

- `Action::CrossEdge(Side)`; `request.rs` ganha `edge esquerda|direita|cima|baixo`.
- Com o app fechado roda sem janela, como `switch_without_window`, mas só periféricos.
- No Linux é o caminho usado pela extensão do GNOME (abaixo).

### Detecção no macOS

- Nova porta `Pointer` (`Send + Sync`): posição do cursor + retângulos de todas as
  telas ativas (`core-graphics`, já dependência: `CGEvent` location e `CGDisplayBounds`).
- Thread no app consultando a cada ~20 ms enquanto Bordas estiver ligado.
- Regras no domínio, testáveis sem sistema:
  - **Borda externa**: o ponto está no limite de uma tela e não há outra tela do outro
    lado naquele ponto (vários monitores no mesmo PC não disparam entre si).
  - **Disparo imediato** (decisão do usuário): o primeiro toque na borda externa de um
    lado com vizinho já troca, sem tempo de espera. Por isso vale só configurar em cima/
    embaixo quem aceita perder teclado e mouse ao encostar na barra de menus ou no Dock;
    a janela mostra esse aviso ao lado dos espaços de cima e de baixo.
  - **Armar/desarmar**: a borda só vale depois que o cursor saiu dela ao menos uma vez
    desde a última travessia ou desde o início. É isso que impede o vai-e-volta: o
    mouse volta e encontra o cursor parado na borda, desarmado.
- Sem permissão de Acessibilidade/Monitoramento de Entrada (só consulta posição).
- **Abre no login** (decisão do usuário): no macOS a borda só funciona com o app aberto,
  então ele passa a abrir sozinho no login, sem janela (só o ícone na barra). Item de
  login pelo `SMAppService` (macOS 13+) ou LaunchAgent em `~/Library/LaunchAgents`
  — escolha do construtor, conferida com a assinatura ad hoc do `bundle.sh`. Sai do
  README a linha de limite "O app não inicia sozinho no login no macOS".

### Detecção no Linux (GNOME/Wayland)

- Wayland não deixa um app ler a posição global do cursor. Uma extensão pequena do
  GNOME Shell (GJS) cria barreiras de pressão (`Meta.Barrier` + `Layout.PressureBarrier`,
  as mesmas dos cantos ativos) nas bordas externas da área de trabalho, sem os cantos.
- Empurrar a barreira roda `kvm-switch edge <lado>`. O app (ou a execução sem janela)
  lê o `config.json` e decide: lado sem vizinho ou Bordas desligado não faz nada.
- A barreira de pressão exige empurrar, então o cursor parado na borda não dispara:
  o vai-e-volta fica resolvido no Linux sem o armar/desarmar.
- A extensão vai em `linux/` e é instalada pelo `install.sh --app` e pelo `deb.sh`
  (pasta de extensões do usuário / do sistema). No Wayland ela só carrega depois de
  sair e entrar na sessão, e o instalador avisa isso.
- **Só GNOME 45 ou mais novo** (decisão do usuário): uma cópia só, em módulos ES
  (`metadata.json` `shell-version` a partir de `"45"`). O `.deb` passa a ser gerado no
  Ubuntu 24.04 (`.github/workflows/release.yml`); README e instaladores dizem o mínimo.
- Quem liga a extensão (escolha minha): o `install.sh --app` roda
  `gnome-extensions enable` para o usuário. Ao ligar "Bordas" na janela, o app faz o
  mesmo e, se a extensão não estiver ativa, mostra a linha "Saia e entre na sessão
  para ativar as bordas" (ou o erro) na janela, como os atalhos recusados.
- Toques repetidos (escolha minha): a extensão ignora novos empurrões enquanto o
  `kvm-switch edge` anterior não terminou e por 1 s depois dele.
- Ícone na barra do Linux e do macOS: sem "Ir para …" com Monitor compartilhado desligado.

### Latência no macOS

- `logi_bolt.rs` procura os 6 slots com um `hidapitester` por pergunta (~2 s).
- Passo 1, cache: guardar, no `LogiBolt`, os slots que responderam e o índice de
  CHANGE_HOST de cada um; nas trocas seguintes mandar só `setCurrentHost` para eles. O
  cache só cresce (escolha minha): depois de cada troca, uma busca completa em segundo
  plano acrescenta slots que não tinham respondido (periférico desligado ou em
  repouso), sem atrasar a troca. Vale também para a troca por atalho.
- Medir antes e depois no hardware; meta: travessia percebida abaixo de ~0,5 s.
- Passo 2, se o cache não chegar à meta (decisão do usuário): trocar o `hidapitester`
  por HID direto no app (crate `hidapi`, receptor aberto uma vez e mantido), com o
  mesmo HID++ do `linux/hidraw.rs`: pergunta o índice, lê a resposta e manda
  `setCurrentHost`, então uma falha passa a ser vista. Novas capturas reais viram
  fixtures. O `hidapitester` sai das dependências do macOS no README e no `bundle.sh`.

### Janela de configuração

- Seção "Modos" com dois interruptores: "Monitor compartilhado (atalhos e seletor)" e
  "Bordas (só teclado e mouse)".
- Quadro 3×3: centro "Este PC"; cada lado mostra o canal (`–`, 1, 2, 3, clique avança,
  como o canal dos destinos) e o nome (clique, digite, Enter, como o nome dos destinos).
- Seletores de teste (`debug_selector`): `"MonitorMode"`, `"EdgesMode"`, `"EdgeChannel(Left)"`,
  `"EdgeName(Left)"` etc., com `#[gpui::test]` cobrindo ligar/desligar e editar um lado.

## Testes esperados

- Domínio: travessia não toca o monitor; sem vizinho/desligado não faz nada; borda
  externa com 1 e 2 telas (lado a lado e empilhadas); canto não dispara; armar/desarmar.
- Config: arquivo de antes dos campos carrega com `monitor_mode = true` e Bordas desligado.
- `request.rs`: `edge` ida e volta.
- UI: os interruptores e a edição de um lado.

## Fica com o construtor

- Nomes exatos de tipos e funções, mantendo português.
- Onde mora a thread de consulta no macOS (`AppState` ou adapter) e como ela para.
- Limiar de pressão da barreira no GNOME e o UUID/nome da extensão.
- Texto exato dos rótulos e mensagens.

## Riscos

- Extensão do GNOME depende da versão do Shell (`metadata.json` `shell-version`); cada
  atualização grande do GNOME pode exigir ajuste.
- Validar no hardware: a meta de latência no macOS e o comportamento dos periféricos em
  repouso ao chegar na travessia.
- Disparo imediato no macOS: encostar sem querer numa borda com vizinho manda teclado e
  mouse embora. Mitigado só pelo armar/desarmar e pelo aviso em cima/embaixo.
- Mudanças em `cfg(target_os)` só compilam no próprio sistema: Linux e macOS precisam ser
  verificados em máquinas separadas.
