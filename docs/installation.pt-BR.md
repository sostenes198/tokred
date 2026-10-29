[English](installation.md) | **Português**

# Instalação

O tokred se instala como o rustup: um comando no terminal baixa o binário da release do GitHub, confere o checksum, coloca na sua pasta pessoal, prepara o seu shell e executa `tokred configure`. Não há gerenciador de pacotes (nada de Homebrew, Scoop, winget ou crates.io), nem `sudo`, nem direitos de administrador.

```sh
# macOS e Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh

# Windows (PowerShell)
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

Os scripts são assets da release (`install.sh`, `install.ps1`, `uninstall.sh`, `uninstall.ps1`). Antes de existir a primeira release, ou para testar a versão da `main`, use a URL raw: `https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.sh` (e `install.ps1`). Os scripts ficam em [`install/`](../install/); leia antes de executar, eles são curtos.

## O que o script faz

1. Detecta a plataforma: macOS ou Linux (musl, então roda em qualquer distribuição) em x86_64 ou aarch64; Windows em x86_64 ou aarch64.
2. Baixa `tokred-<target>.tar.gz` (Windows: `.zip`) e o `.sha256` da release e **confere o checksum**. Se não bater, tudo para antes de qualquer coisa ser gravada.
3. Instala o binário em `${TOKRED_HOME:-$HOME/.tokred}/bin` (Windows: `%LOCALAPPDATA%\tokred\bin`).
4. Adiciona essa pasta ao `PATH` e define o [wrapper `claude`](#o-wrapper-claude): nos perfis de shell que existem (`~/.bashrc`, `~/.bash_profile`, `~/.zshrc`, mais `~/.profile` e, com o fish instalado, `~/.config/fish/conf.d/tokred.fish`); no Windows, no `PATH` do usuário e no perfil do PowerShell. Tudo o que ele grava fica em um único bloco marcado (`# >>> tokred >>>` ... `# <<< tokred <<<`), que é substituído a cada execução (rodar o script duas vezes não muda nada) e removido pelo script de desinstalação.
5. Executa o [`tokred configure`](commands.pt-BR.md), que integra o tokred ao Claude Code e faz as duas perguntas abaixo.

Ele nunca usa `sudo`, nunca grava fora da pasta do tokred e dos seus arquivos de perfil, e se recusa a rodar como root, a menos que você passe `--yes`. Com `curl ... | sh` o stdin do script é o pipe, então as perguntas são feitas no terminal (`/dev/tty`); sem terminal nenhum, ele aplica os padrões.

### As duas perguntas

- **Qual porta o proxy usa?** Padrão `1029` (ou a que já está no `tokred.toml`). Só são aceitas portas de 1024 a 65535; o instalador confere se a porta está livre tentando abrir `127.0.0.1:<porta>`, avisa quando está ocupada e pergunta de novo. Se o proxy desta instalação já escuta nela, está tudo certo.
- **Gravar o conteúdo das conversas para o painel mostrar o que foi enviado e devolvido? [Y/n]** O texto fica no banco de dados do próprio tokred, na sua máquina; cabeçalhos e credenciais nunca são gravados. A resposta é salva como `[proxy] log_bodies` no `tokred.toml`. Sem terminal, ou com `--yes`, o conteúdo **não** é gravado, a menos que você diga o contrário (`--record-content`, `TOKRED_RECORD_CONTENT=1`). Mude depois editando o `tokred.toml` ou rodando `tokred configure` de novo.

### Opções

O `irm | iex` não consegue passar parâmetros, então toda opção também tem uma variável de ambiente (as mesmas variáveis valem para o `sh`).

| Opção do shell | PowerShell | Variável de ambiente | Significado |
|---|---|---|---|
| `-y`, `--yes` | `-Yes` | `TOKRED_YES=1` | Não pergunta nada; usa os padrões e as opções abaixo |
| `--port N` | `-Port N` | `TOKRED_PORT` | Porta do proxy (1024 a 65535) |
| `--version X` | `-Version X` | `TOKRED_VERSION` | Instala a release `vX` em vez da mais recente |
| `--no-modify-path` | `-NoModifyPath` | `TOKRED_NO_MODIFY_PATH=1` | Não mexe no `PATH` |
| `--no-alias` | `-NoAlias` | `TOKRED_NO_ALIAS=1` | Não define o wrapper `claude` |
| `--no-configure` | `-NoConfigure` | `TOKRED_NO_CONFIGURE=1` | Só instala o binário |
| | | `TOKRED_HOME` | Pasta de instalação (padrão `~/.tokred`, Windows `%LOCALAPPDATA%\tokred`) |
| | | `TOKRED_RELEASE_BASE_URL` | De onde baixar os assets da release (`https://` ou `file://`); para espelhos e instalação offline |

`--no-modify-path` e `--no-alias` são independentes: o primeiro pula o `PATH`, o segundo pula o wrapper; com os dois, os perfis não são tocados. Exemplos:

```sh
curl --proto '=https' --tlsv1.2 -LsSf .../install.sh | sh -s -- --yes --port 2000 --no-alias
TOKRED_VERSION=0.2.0 TOKRED_NO_ALIAS=1 sh install.sh
```

```powershell
$env:TOKRED_PORT = '2000'; $env:TOKRED_NO_ALIAS = '1'; irm .../install.ps1 | iex
powershell -File install.ps1 -Yes -Port 2000          # ou execute o arquivo baixado com parâmetros
```

## O wrapper `claude`

O instalador adiciona uma função de shell chamada `claude` que executa `tokred initialize` e depois o Claude Code de verdade, com os seus argumentos intactos:

```sh
claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }
```

O `tokred initialize` é silencioso e rápido, e sempre sai com código 0: ele faz as configurações do Claude Code concordarem com a chave do proxy (`[proxy] enabled`): com o proxy ligado, aponta o Claude Code para ele (e o inicia, se preciso); desligado, remove só a `ANTHROPIC_BASE_URL` do próprio tokred e deixa em paz outro gateway seu. Assim, `claude` sempre faz o que a chave diz, em todo terminal.

- **É fail-open.** Se o `tokred` não existir (desinstalado, movido), o `claude` inicia normalmente.
- **Para ignorá-lo:** `command claude` executa o binário real sem o wrapper (PowerShell: `& (Get-Command claude -CommandType Application)`).
- **Para ligar ou desligar o proxy:** `tokred proxy disable` / `tokred proxy enable`.
- **Para não instalá-lo:** `--no-alias` / `-NoAlias` / `TOKRED_NO_ALIAS=1`. Para remover depois, apague o bloco entre os marcadores `# >>> tokred >>>` ou execute o script de desinstalação.
- **Shells cobertos:** bash, zsh, fish e PowerShell (Windows PowerShell e pwsh). O `cmd.exe` não tem aliases persistentes, então não é coberto; inicie o Claude Code pelo PowerShell ou rode `tokred initialize` à mão antes.
- **É uma função, não um alias,** para repassar os argumentos e não poder chamar a si mesma. O bloco executa `unalias claude` antes, porque o instalador do próprio Claude Code pode adicionar `alias claude=...` ao `~/.zshrc`; se esse alias era o único caminho até o `claude`, o `command claude` não o encontrará. Coloque o `claude` real no `PATH`.
- **PowerShell** precisa que um perfil seja carregado: se janelas novas não recebem o wrapper, a política de execução pode ser `Restricted`; rode `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` uma vez.

## Instalar uma versão específica, offline ou de um espelho

```sh
sh install.sh --version 0.2.0
TOKRED_RELEASE_BASE_URL=https://mirror.example/tokred/v0.2.0 sh install.sh
TOKRED_RELEASE_BASE_URL=file:///srv/tokred sh install.sh      # uma pasta com o arquivo e o .sha256
```

A pasta (ou URL) precisa conter `tokred-<target>.tar.gz` / `.zip` e `tokred-<target>.tar.gz.sha256` / `.zip.sha256`; o checksum é sempre conferido. Para conferir um download à mão: `shasum -a 256 tokred-*.tar.gz` (Windows: `Get-FileHash`) e compare com o arquivo `.sha256`.

## Compilando do código-fonte

Para desenvolvimento: `cargo build` e execute `target/debug/tokred`, veja [testes](testing.pt-BR.md). Um binário compilado assim (ou colocado fora da pasta de instalação) funciona, mas não se atualiza sozinho: o `tokred update` manda reinstalar com o script.

## Opcional: o proxy local

O [proxy](proxy.pt-BR.md) é opcional e fica desligado até você usá-lo. Nada é gravado nas configurações do Claude Code a menos que você execute um comando que diga isso.

```sh
tokred claude                     # inicia o proxy se preciso e executa o Claude Code por ele, só nesta execução
tokred proxy service install      # opcional: inicia o proxy no login, como serviço de usuário
```

`tokred compression on` é o único comando que edita a configuração de usuário do Claude Code (`~/.claude.json`, com backup em `.tokred.bak`), e só quando você o executa.

## Atualização

O `tokred update` olha onde o binário em execução está:

| Onde o binário está | O que `tokred update` faz |
|---|---|
| Na pasta de instalação (`$TOKRED_HOME/bin`), colocado pelo script | Baixa o arquivo desta plataforma da release do GitHub, **confere o SHA-256**, troca o binário de forma atômica (no Windows o `.exe` em execução é renomeado para `.exe.old` e removido no próximo início) e reaplica a configuração com o binário novo |
| Em qualquer outro lugar (compilado do código-fonte, copiado à mão) | Avisa que existe versão nova e manda reinstalar com o script de instalação |

Rodar o script de instalação de novo faz o mesmo trabalho e também serve para um binário que não consegue se atualizar. Depois que o binário muda de versão, o primeiro comando da CLI reaplica a configuração de forma não interativa (hooks nunca fazem isso); suas escolhas de porta e de gravação de conteúdo são mantidas. O `tokred doctor` mostra quando há reconfiguração pendente.

## Verificação de versão

Uma vez por dia, só a partir de comandos da CLI (nunca de um hook), o tokred consulta o GitHub pela última release com timeout de 2 s e, se houver versão mais nova, imprime um aviso de uma linha no **stderr**, para não afetar a saída `--json`. Desligue com `[update] mode = "off"` ou `TOKRED_NO_UPDATE_CHECK=1`; `TOKRED_OFFLINE=1` desliga toda chamada de rede (verificação de versão e sincronização de preços).

## Desinstalação

```sh
# macOS e Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/uninstall.sh | sh

# Windows (PowerShell)
irm https://github.com/sostenes198/tokred/releases/latest/download/uninstall.ps1 | iex
```

(Ou `sh install/uninstall.sh` a partir de um clone.) Opções: `-y/--yes` (`-Yes`, `TOKRED_YES=1`) não pergunta nada e mantém seus dados; `--purge` (`-Purge`, `TOKRED_PURGE=1`) também apaga a pasta de dados do tokred. Sem elas, ele pergunta.

O script executa primeiro o `tokred uninstall`, que:

1. para o proxy e remove o serviço de usuário dele, se instalado;
2. desliga a compressão, se estava ligada;
3. remove os hooks e agentes do tokred do Claude Code (`tokred unconfigure`; agentes que você editou são mantidos);
4. pergunta se a pasta de dados do tokred (banco, config, logs, relatórios) deve ser apagada, a menos que você tenha passado `--yes` ou `--purge`.

Cada passo é feito no melhor esforço: uma falha é informada e o passo seguinte continua. Se você responder "não" à confirmação, nada é removido. Depois o script remove o binário (no Windows só depois que o `tokred.exe` terminou: um programa em execução não consegue se apagar), a pasta de instalação se estiver vazia, a entrada no `PATH` e os blocos do wrapper `claude` (um arquivo fish que contém só o nosso bloco é apagado; um com outras linhas as mantém). O `tokred uninstall` também pode ser executado sozinho; ele então mostra o que sobrou para o script.

## Publicando uma release (mantenedores)

Envie uma tag `vX.Y.Z`; o `release.yml` gerado constrói os arquivos e envia para a release do GitHub, junto com seus `.sha256` e os quatro scripts de instalação. Não são necessários tokens, nem repositórios de tap ou bucket. Passo a passo: [release](release.pt-BR.md).
