[English](installation.md) | **Português**

# Instalação

Como instalar, atualizar e remover o tokred, e o que o instalador altera na sua máquina.

## Instalar

**macOS e Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

Os scripts são assets da release e ficam em [`install/`](../install/); leia-os antes de executar. Não é preciso `sudo` nem direitos de administrador.

### Uma versão específica ou uma pré-release

Os comandos acima instalam a release estável mais recente. Uma pré-release só é instalada quando você informa a versão dela. Troque `X.Y.Z` por uma versão da [página de releases](https://github.com/sostenes198/tokred/releases), como o número de uma tag de release (um `v` inicial é opcional). O script da `main` instala qualquer versão.

**macOS e Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.sh | sh -s -- --version X.Y.Z
```

**Windows (PowerShell)**

```powershell
$env:TOKRED_VERSION = 'X.Y.Z'; irm https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.ps1 | iex
```

## O que o instalador faz

1. Detecta sua plataforma: macOS ou Linux (musl, então roda em qualquer distribuição) em x86_64 ou aarch64, ou Windows em x86_64 (o Windows em ARM roda esse build por emulação).
2. Baixa `tokred-<target>.tar.gz` (Windows: `.zip`) e o `.sha256` dele da release do GitHub e **verifica o checksum**. Uma divergência interrompe tudo antes de qualquer coisa ser gravada.
3. Instala o binário em `${TOKRED_HOME:-$HOME/.tokred}/bin` (Windows: `%LOCALAPPDATA%\tokred\bin`).
4. Adiciona essa pasta ao seu `PATH` e define o [wrapper `claude`](#o-wrapper-claude) nos seus perfis de shell: sempre o `~/.profile`; o `~/.bashrc`, o `~/.bash_profile` e o `~/.zshrc` quando existem (o `~/.zshrc` também é criado quando seu shell é o zsh; o `$ZDOTDIR` é respeitado); e o `~/.config/fish/conf.d/tokred.fish` quando o fish está instalado. No Windows usa o `PATH` do seu usuário e o seu perfil do PowerShell (`profile.ps1` em `Documents\WindowsPowerShell` e, quando o PowerShell 7 está presente, em `Documents\PowerShell`). Tudo o que ele grava fica em um único bloco marcado (`# >>> tokred >>>` ... `# <<< tokred <<<`) que é substituído a cada execução, então rodar o script duas vezes não muda nada.
5. Executa `tokred configure`, que adiciona os hooks e agentes do tokred ao Claude Code (veja [hooks e agentes](hooks-and-agents.pt-BR.md)).
6. Executa `tokred proxy enable`, que faz as duas perguntas abaixo, salva as respostas, inicia o [proxy](proxy.pt-BR.md) e aponta o Claude Code para ele. Esse passo é pulado quando o `configure` é recusado ou falha.

O script nunca grava fora da pasta do tokred e dos seus arquivos de perfil, e se recusa a rodar como root a menos que você passe `--yes`. Com `curl ... | sh` ele faz as perguntas no terminal (`/dev/tty`); sem terminal, aplica os padrões.

**Qual porta o proxy usa?** O padrão é `1029`, ou a que já estiver no `tokred.toml`. Só são aceitas portas de 1024 a 65535. Em um terminal, o instalador pergunta de novo quando a porta está ocupada; uma porta informada com `--port` é usada como está, com um aviso, e o proxy inicia quando a porta ficar livre.

**Gravar o conteúdo das conversas para o dashboard poder mostrá-lo?** O texto fica no banco de dados do próprio tokred, na sua máquina; cabeçalhos e credenciais nunca são gravados. A resposta é salva como `[proxy] log_bodies`. Em um terminal, a resposta padrão é sim, ou a que você deu antes. Sem terminal, ou com `--yes`, o conteúdo não é gravado, a menos que uma resposta salva diga que sim. Mude depois editando o `tokred.toml` ou executando `tokred proxy enable` de novo.

## Opções

`irm | iex` não consegue passar parâmetros, então toda opção também tem uma variável de ambiente. As mesmas variáveis funcionam com `sh`.

| Opção do shell | PowerShell | Variável de ambiente | Significado |
|---|---|---|---|
| `-y`, `--yes` | `-Yes` | `TOKRED_YES=1` | Não pergunta nada; usa os padrões. Também permite rodar como root |
| `--port N` | `-Port N` | `TOKRED_PORT` | Porta do proxy (1024 a 65535) |
| `--version X` | `-Version X` | `TOKRED_VERSION` | Instala a release `vX` em vez da mais recente |
| `--no-modify-path` | `-NoModifyPath` | `TOKRED_NO_MODIFY_PATH=1` | Não mexe no `PATH` |
| `--no-alias` | `-NoAlias` | `TOKRED_NO_ALIAS=1` | Não define o wrapper `claude` |
| `--no-proxy` | `-NoProxy` | `TOKRED_NO_PROXY=1` | Só hooks e agentes; não executa `tokred proxy enable` |
| `--no-configure` | `-NoConfigure` | `TOKRED_NO_CONFIGURE=1` | Só instala o binário (sem `configure`, sem proxy) |
| | | `TOKRED_HOME` | Pasta de instalação (padrão `~/.tokred`; no Windows, `%LOCALAPPDATA%\tokred`) |
| | | `TOKRED_RELEASE_BASE_URL` | De onde os assets da release são baixados (`https://` ou `file://`), para espelhos e instalações offline |

`--no-modify-path` e `--no-alias` são independentes; com os dois, seus perfis não são tocados. `--port` não tem efeito com `--no-proxy` ou `--no-configure`. Exemplos:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh -s -- --yes --port 2000 --no-alias
```

```powershell
$env:TOKRED_PORT = '2000'; $env:TOKRED_NO_ALIAS = '1'; irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

Para uma instalação offline, aponte `TOKRED_RELEASE_BASE_URL` para uma pasta (ou URL) que contenha `tokred-<target>.tar.gz` / `.zip` e o respectivo arquivo `.sha256` (o `--version` é ignorado nesse caso); o checksum é sempre verificado.

## O wrapper `claude`

O instalador adiciona uma função de shell chamada `claude` que executa `tokred initialize` e depois o Claude Code real, com seus argumentos intactos:

```sh
claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }
```

`tokred initialize` é silencioso e rápido, e sempre sai com código 0. Ele faz as configurações do Claude Code concordarem com a chave do proxy (`[proxy] enabled`): com o proxy ligado, aponta o Claude Code para ele (e o inicia se preciso); com o proxy desligado, remove apenas o `ANTHROPIC_BASE_URL` do próprio tokred e deixa em paz um gateway seu.

| Você quer | Faça isto |
|---|---|
| Rodar o `claude` real uma vez | `command claude` (PowerShell: `& (Get-Command claude -CommandType Application)`) |
| Desligar ou ligar o proxy | `tokred proxy disable` / `tokred proxy enable` |
| Não instalar o wrapper | `--no-alias`, `-NoAlias` ou `TOKRED_NO_ALIAS=1` |
| Remover o wrapper depois | Apague o bloco entre os marcadores `# >>> tokred >>>`, ou [desinstale](#desinstalar) |

O wrapper é fail-open: se o `tokred` não existir, o `claude` inicia normalmente. Ele cobre bash, zsh, fish e PowerShell; o `cmd.exe` não tem aliases persistentes, então inicie o Claude Code pelo PowerShell ou execute `tokred initialize` manualmente antes. O instalador do próprio Claude Code pode adicionar `alias claude=...` ao `~/.zshrc`; o bloco executa `unalias claude` primeiro, então coloque o `claude` real no seu `PATH` se esse alias era a única forma de alcançá-lo.

## Atualizar

```sh
tokred update
```

```sh
tokred update --version X.Y.Z
```

O `tokred update` olha onde o binário em execução está. Na pasta de instalação (colocado lá pelo script), ele baixa o arquivo da sua plataforma, verifica o SHA-256, substitui o binário e reaplica a configuração. Em qualquer outro lugar (compilado do código-fonte, copiado à mão), ele avisa que existe uma versão nova e pede que você reinstale com o script, que faz o mesmo trabalho. Sem `--version`, instala a release estável mais recente; com ele, exatamente aquela versão (uma pré-release ou uma mais antiga). Sua porta e sua escolha sobre gravar conteúdo são mantidas. Quando o binário muda de versão por outros meios (por exemplo, um novo `cargo install`), o primeiro comando `doctor`, `stats`, `metrics`, `audit`, `pricing` ou `cache` reaplica a configuração em silêncio.

## Desinstalar

**macOS e Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/uninstall.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/sostenes198/tokred/releases/latest/download/uninstall.ps1 | iex
```

A partir de um clone, execute `sh install/uninstall.sh`. Ele remove tudo **sem perguntar**, executando `tokred uninstall`, que:

1. para o proxy e remove o serviço de usuário dele, se instalado;
2. desliga a compressão, se estava ligada;
3. remove o `ANTHROPIC_BASE_URL` do tokred das configurações do Claude Code (um gateway seu permanece);
4. remove os hooks e agentes do tokred do Claude Code (agentes que você editou são mantidos e informados);
5. apaga a pasta de dados do tokred (banco de dados, configuração, logs, relatórios);
6. remove o binário, a pasta de instalação (quando não há mais nada nela), a entrada no `PATH` e os blocos do wrapper `claude` nos seus perfis de shell (um arquivo do fish que só tem o bloco do tokred é apagado). Um binário compilado do código-fonte é deixado onde está. No Windows, essa última parte roda assim que o `tokred.exe` termina, porque um programa em execução não consegue se apagar.

Cada passo é de melhor esforço: uma falha é informada e o passo seguinte ainda roda, e se algum passo falhou o binário é mantido para você poder executar o comando de novo. O `tokred uninstall` também pode ser executado sozinho. Sem o binário, o script limpa o que sobrou nos seus perfis de shell.

## Compilar a partir do código-fonte

Você precisa do Rust 1.89 ou mais novo.

```sh
cargo install --path crates/tokred-cli
```

Isso coloca o `tokred` na pasta bin do Cargo, que precisa estar no seu `PATH`. Depois execute `tokred configure` para os hooks e agentes, e `tokred proxy enable` se quiser o proxy; a compilação não faz nenhum dos dois. Um binário compilado assim não se atualiza sozinho: o `tokred update` pede que você reinstale. Para trabalhar no tokred, veja [desenvolvimento](development.pt-BR.md).

## Resolução de problemas

| Problema | O que fazer |
|---|---|
| `tokred: command not found` depois de instalar | Abra um novo terminal para carregar o novo `PATH`, ou confira se `~/.tokred/bin` (Windows: `%LOCALAPPDATA%\tokred\bin`) está nele. Sem `--no-modify-path`, o instalador o adiciona. |
| O wrapper `claude` não aparece em novas janelas do PowerShell | Sua política de execução pode ser `Restricted`. Execute `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` uma vez. |
| A porta do proxy está ocupada | Escolha outra: `tokred proxy enable --port 2000`, ou instale com `--port 2000`. |
| O proxy não alcança a API a partir de uma rede corporativa | O proxy respeita `HTTPS_PROXY`, `NO_PROXY` e afins; veja [proxy](proxy.pt-BR.md). |
| Outra coisa parece errada | Execute `tokred doctor`; ele mostra o que está instalado e o que precisa de atenção. |
