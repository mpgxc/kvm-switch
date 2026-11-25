# Área de transferência junto com a troca pela borda

## Intenção

Na troca pela borda, teclado e mouse passam para o PC vizinho, mas o que foi copiado
fica para trás. O usuário quer copiar num PC, levar o cursor até a borda e colar no
outro, como no Logitech Flow. Isso exige rede: hoje a borda só troca o canal do
receptor Bolt e o desenho dela deixou a área de transferência de fora por esse
motivo (`2026-11-24-troca-pela-borda-design.md`, "Não objetivos").

## Decisões do usuário

- **macOS, Windows e Linux (GNOME) já na primeira versão.**
- **Texto e imagens** já na primeira versão.
- **Senhas copiadas de gerenciadores vão como qualquer texto** (cifradas): copiar a
  senha num PC e colar no outro é um uso desejado. A marca de conteúdo sigiloso dos
  gerenciadores não é consultada.

## Objetivos

1. Com o modo Bordas e a área de transferência ligados, cruzar a borda de um lado com
   vizinho leva o texto ou a imagem copiada para a área de transferência do vizinho;
   lá o usuário cola com Ctrl/⌘+V.
2. Funciona entre quaisquer dois dos três sistemas, nos dois sentidos, e numa fileira
   de três PCs (A | B | C: o que veio de A segue de B para C).
3. O conteúdo viaja cifrado e só PCs com a mesma chave conseguem gravar.

## Não objetivos

- Texto com formatação (HTML/RTF) e arquivos copiados.
- Descobrir os PCs sozinho na rede: o endereço do vizinho é digitado.
- Passar pelo hub do Orange Pi ou por qualquer servidor.
- Sincronizar a cada cópia. Só a travessia leva o conteúdo.
- Levar a área de transferência pelo atalho do Monitor compartilhado.

## Anti-objetivos (falha mesmo funcionando)

- A travessia ficar mais lenta, ou deixar de acontecer, por causa da rede. Vizinho
  desligado, endereço errado, chave errada ou imagem grande nunca seguram teclado e
  mouse.
- Conteúdo copiado passar aberto pela rede, ou quem não tem a chave conseguir gravar na
  área de transferência de outro PC (inclusive repetindo um envio capturado).
- Uma atualização mudar o comportamento de quem já usa o app: `config.json` antigo
  carrega com a área de transferência **desligada**.
- Voltar ao PC de origem e encontrar a cópia original trocada por uma versão
  convertida dela mesma.

## Restrições do projeto (de `AGENTS.md`)

- Português em identificadores, comentários, textos e erros; testes em inglês.
- Hexagonal: regras em `domain.rs`, contratos em `ports.rs`, sistema em `adapters/`.
- Campos novos de `Config` com `#[serde(default)]` e teste de arquivo antigo.
- Lints do `Cargo.toml`; `cargo test` e o clippy em cada sistema alterado; `README.md`
  (tabela de arquivos, uso, limites) e `AGENTS.md` atualizados.
- Invariante que continua valendo: a travessia nunca toca o monitor.

## Desenho (escolhas minhas, ajustáveis)

### Quem manda e quando

Manda o PC que o cursor está deixando: é o único que sabe da travessia, porque o PC
que recebe os periféricos não percebe a troca de canal. É o mesmo modelo do Deskflow.

```
cursor na borda → AppState::cross / cross_without_window
                   ├─ Kvm::cross (Bolt, como hoje)
                   └─ em paralelo, outra thread: lê o conteúdo → envia ao vizinho
vizinho: escuta → confere chave, hora e nonce → igual ao atual? nada : grava
```

- O envio roda em `std::thread` própria, iniciado junto com `Kvm::cross`. Prazo de
  300 ms para conectar e 5 s para escrever (uma imagem de 16 MiB no Wi-Fi).
- **Sem memória no lado de quem envia:** toda travessia envia o conteúdo atual. Quem
  recebe compara com o que já tem (mesmo tipo e mesmos bytes, lidos pela própria
  porta `Copia`) e, se for igual, não grava. Isso protege a cópia original na volta ao
  PC de origem, funciona com três PCs em fileira e não exige estado compartilhado
  entre o app, a execução sem janela e o processo do ícone no Linux.
- Para essa comparação valer, cada adapter tem de cumprir: **gravar um conteúdo e lê-lo
  de volta devolve os mesmos bytes** (imagem gravada como PNG é lida como esse PNG).
- Área de transferência sem texto nem imagem: não envia, e o vizinho fica com o que
  tinha. Se o sistema oferece uma forma em texto de outro conteúdo (o Finder põe o
  nome do arquivo copiado), essa forma vai.
- `cross_without_window` (a extensão do GNOME rodando `kvm-switch edge` com o app
  fechado) também envia, e espera o envio terminar antes de sair (no pior caso
  ~5,3 s, durante os quais a extensão ignora novos empurrões; a pausa dela já é 1 s).

### Conteúdo

```rust
pub enum Conteudo {
    Texto(String),
    /// Sempre PNG na rede; cada adapter converte do formato do sistema.
    ImagemPng(Vec<u8>),
}
```

- Com texto e imagem ao mesmo tempo, vai **o texto** (o Word e o Excel põem uma imagem
  da seleção junto do texto; quem copia ali quer colar o texto).
- Limite de **16 MiB** por envio. Acima disso não envia (stderr).

### Avisos

- Quem envia não mostra nada: o usuário já está olhando o outro PC. Falhas (vizinho
  desligado, endereço errado, conteúdo acima do limite) vão para o stderr.
- Quem recebe avisa onde o usuário está olhando quando **recusa** uma mensagem: chave
  diferente ou relógio fora da janela. No macOS e no Windows, pelo HUD do app; no
  Linux, por uma notificação do GNOME (`Main.notify`, pela extensão). No máximo um
  aviso por minuto, para ninguém na rede conseguir inundar a tela.
- Não há resposta pela rede: o protocolo é só de ida.

### Ler e gravar em cada sistema

Nova porta em `ports.rs`, chamada fora da thread do GPUI (a leitura na thread do envio
e a gravação na thread que ouve):

```rust
pub trait Copia: Send + Sync {
    /// O que está copiado agora; `None` sem texto nem imagem.
    fn ler(&self) -> Result<Option<Conteudo>>;
    fn gravar(&self, conteudo: &Conteudo) -> Result<()>;
}
```

| Sistema | Texto | Imagem |
|---|---|---|
| macOS: `NSPasteboard.generalPasteboard` via `objc2-app-kit` (já no `Cargo.lock`) | `stringForType`/`setString` | Lê `public.png`; só TIFF ⇒ converte com `NSBitmapImageRep` para PNG. Grava `public.png`. |
| Windows: `windows-sys` já usado, mais as features `Win32_System_DataExchange`, `Win32_System_Memory`, `Win32_System_Ole` | `CF_UNICODETEXT` | Lê o formato registrado `"PNG"`; só `CF_DIB`/`CF_DIBV5` (Win+Shift+S, Paint) ⇒ converte para PNG com o crate `png` (já dependência). Grava `"PNG"` e também `CF_DIB`, porque vários programas só colam DIB. |
| Linux (GNOME): D-Bus para a extensão, com `zbus` (já no `Cargo.lock` no Linux, via `ksni` e `ashpd`) | `St.Clipboard` `text/plain;charset=utf-8` | `St.Clipboard` `image/png` (`get_mimetypes`, `get_content`, `set_content`). |

- macOS: não `pbpaste`/`pbcopy`: `adapters::run` não escreve no stdin, eles não levam
  imagem e, abertos pelo LaunchAgent sem `LANG`, podem estragar acentos.
- Windows: sem janela dona, conferir que `SetClipboardData` aceita após
  `OpenClipboard(NULL)`; se não, uma janela de mensagens oculta.
- Linux: no Wayland, app em segundo plano não lê nem grava a área de transferência e o
  Mutter não tem data-control. A extensão roda dentro do gnome-shell. Vale também no
  X11, para ter um caminho só.
- Não uso `cx.read_from_clipboard` do GPUI: ele exige a thread principal e não funciona
  no Linux sem foco; a porta acima funciona igual com o app aberto ou sem janela.

### Extensão do GNOME

- Exporta, com `Gio.DBusExportedObject.wrapJSObject`, um objeto em
  `/io/github/mpgxc/KvmSwitch` na conexão de sessão do gnome-shell
  (`org.gnome.Shell`), interface `io.github.mpgxc.KvmSwitch` (nomes à escolha do
  construtor):
  - `Ler() → (s tipo, ay dados)` (assíncrono; tipo `texto`, `png` ou vazio; texto
    preferido quando há os dois);
  - `Gravar(s tipo, ay dados)`;
  - `Avisar(s)` (`Main.notify`).
- Desexporta no `disable()`.
- O fluxo da borda não muda: empurrar a barreira continua rodando
  `kvm-switch edge <lado>`, e quem lê a área de transferência é o kvm-switch, pela
  porta `Copia`.
- Extensão desligada ou fora do ar: `ler`/`gravar` falham com o erro do D-Bus (stderr).

### Quem ouve a porta

- macOS e Windows: o app, que já abre no login (LaunchAgent e atalho de
  inicialização com `start`, sem janela).
- Linux: o processo do ícone (`kvm-switch tray`, `adapters/linux/tray.rs`), que é o
  que abre no login (autostart do `install.sh --app` e do `.deb`); o app GPUI no
  Linux pode estar fechado. Ele já relê o `config.json` a cada 2 s e passa a ligar e
  desligar a escuta conforme a configuração. O app GPUI no Linux não ouve, para não
  disputar a porta.

### Rede

- TCP direto, porta fixa **24811** (escolha minha, ao lado da 24800 do Deskflow),
  ouvindo em todas as interfaces só enquanto a área de transferência estiver ligada.
- Uma conexão por envio, só de ida. O vizinho de cada lado ganha um endereço: nome
  `.local` (mDNS: macOS, Ubuntu com avahi, Windows 10+) ou IP. Sem endereço, aquele
  lado não envia.
- Quadro: `"KVMC"` + versão `u8` + nonce de 12 bytes + tamanho `u32` + cifrado.
- Cifra: ChaCha20-Poly1305 do `ring` (já no `Cargo.lock`, via o cliente HTTP do GPUI),
  cabeçalho como dado associado, nonce aleatório (`ring::rand::SystemRandom`).
- Dentro do cifrado: hora do envio (ms Unix) + tipo `u8` (1 texto UTF-8, 2 PNG) +
  dados.
- Quem recebe descarta, sem responder: chave errada, versão ou tipo desconhecido,
  tamanho acima de **16 MiB**, hora com mais de 30 s de diferença do próprio relógio,
  ou nonce já visto nos últimos 30 s (guarda os nonces da janela, para um envio
  capturado não poder ser repetido).
- Leitura com prazo de 5 s; uma conexão por vez basta.

### Chave

- 32 bytes aleatórios, mostrados em base64. A mesma em todos os PCs.
- Ao ligar a área de transferência com a chave vazia, o app gera uma. Na janela: o
  campo da chave, com botão "Copiar" e edição (cola-se a chave do outro PC e Enter).
- Guardada no `config.json`. No macOS e no Linux, o arquivo passa a ser salvo com
  permissão `0600` (escolha minha); no Windows, `%APPDATA%` já é do usuário.

### Configuração

```jsonc
{
  "edges": {
    "enabled": true,
    "left": { "channel": 2, "name": "Linux", "address": "pc-linux.local" }, // novo
    ...
  },
  "clipboard": { "enabled": false, "key": "" }                              // novo
}
```

Nomes exatos à escolha do construtor, seguindo os já serializados. Ausentes ⇒ sem
endereço e desligado.

### Janela de configuração

- Em "Modos", terceiro interruptor: "Área de transferência (junto com as bordas)".
  Só tem efeito com Bordas ligado; com Bordas desligado fica visível e apagado.
- No quadro 3×3, cada lado ganha o endereço (clique, digite, Enter, como o nome).
- Campo da chave com "Copiar".

### Regras no domínio (testáveis sem sistema)

- Montar e abrir o quadro, com texto e com PNG: ida e volta; chave errada, quadro
  adulterado, hora fora da janela, nonce repetido, versão ou tipo desconhecido e
  tamanho acima do limite são recusados.
- Receber conteúdo igual ao atual não grava; texto e imagem com os mesmos bytes não
  contam como iguais.
- Lado sem endereço, área de transferência desligada ou Bordas desligado: não envia.
- Limite de avisos de recusa: um por minuto.
- `Config` antigo carrega com a área de transferência desligada e sem endereços.

## Testes esperados

- Domínio: os itens acima.
- Rede: dois sockets locais (`127.0.0.1`, porta 0) trocando texto e PNG com uma
  `Copia` falsa; chave errada não grava; o mesmo quadro enviado duas vezes grava uma.
- Conversões: DIB → PNG no Windows a partir de uma captura real (fixture com a origem
  anotada, como as outras).
- UI: o interruptor novo, a edição do endereço e da chave (`debug_selector`
  `"ClipboardMode"`, `"EdgeAddress(Left)"`, `"ClipboardKey"`).
- Manual, no hardware, nos dois sentidos de macOS ↔ Windows, macOS ↔ Linux e
  Windows ↔ Linux:
  - texto com acentos e emoji;
  - print da tela (⌘⇧⌃4, Win+Shift+S, PrtSc do GNOME) e imagem copiada do navegador,
    colados no Paint, no Preview e no GIMP/LibreOffice;
  - ir e voltar não troca a cópia original;
  - vizinho desligado não atrasa a travessia.

## Instalação e firewall

- Windows: o instalador é por usuário (`PrivilegesRequired=lowest`) e não pode criar
  regra de firewall. Fica o pedido do próprio Windows na primeira escuta (escolha
  minha: não mudar o instalador para administrador); o README explica.
- macOS: o firewall, se ligado, pergunta na primeira escuta. Suposição não testada: a
  assinatura ad hoc do `bundle.sh` pode fazer a pergunta voltar a cada versão.
- Linux: o Ubuntu vem sem firewall ativo; com `ufw` ligado, o README dá o comando.

## Fica com o construtor

- Nomes de tipos, funções, métodos D-Bus e campos.
- Onde mora a thread que ouve e como ela para ao desligar o modo.
- Como o aviso de recusa chega ao HUD do app no macOS/Windows (por exemplo, uma
  `Action` nova sem dados).
- Converter imagem no macOS com `NSBitmapImageRep` ou outro caminho do AppKit.
- Texto exato dos rótulos e mensagens.

## Riscos

- macOS: a Apple anuncia como futuro um alerta para apps que leem a área de
  transferência sem o usuário colar (`NSPasteboard.accessBehavior`, macOS 15.4). Quando
  chegar, a primeira leitura deve perguntar uma vez.
- Senhas vão para o vizinho (cifradas), por decisão do usuário. Ficam na área de
  transferência dele até a próxima cópia, e o histórico de área de transferência do
  sistema de destino (Win+V, gerenciadores do GNOME) pode guardá-las.
- Imagem grande no Wi-Fi: um print 4K em PNG tem alguns MB. Se não chegar antes do
  Ctrl+V, o usuário cola o conteúdo antigo; colar de novo resolve.
- GNOME: a extensão pode quebrar a cada versão grande do Shell (já vale para a borda).
  Os métodos D-Bus ficam acessíveis a qualquer processo da sessão do usuário.
- Relógios: a janela de 30 s exige relógios sincronizados (NTP, o padrão nos três).
- Endereço por IP muda com o DHCP; `.local` ou IP fixo no roteador resolvem.
- `cfg(target_os)`: cada sistema precisa ser compilado e testado na própria máquina.
