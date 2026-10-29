[English](release.md) | **Português**

# Publicando o tokred

Para mantenedores: como uma versão é cortada, testada e publicada. Os usuários instalam a partir do release publicado ([installation](installation.pt-BR.md)); quem contribui compila e testa a partir do código-fonte ([development](development.pt-BR.md)).

## O fluxo

Uma versão passa por um release candidate (RC), é testada já publicada e só então vira a versão final.

```text
scripts/release.sh rc 0.2.0-rc.1 ──► tag v0.2.0-rc.1 ──► workflow Release ──► Release smoke
                                                                                    │ verde
scripts/release.sh promote 0.2.0-rc.1 ──► tag v0.2.0 ──► workflow Release ──► Release smoke
```

1. **Cortar o RC.** `scripts/release.sh rc 0.2.0-rc.1` sobe a versão, move as entradas do changelog para baixo dela, faz o commit, cria a tag `v0.2.0-rc.1` e dá push.
2. **Compilar.** A tag inicia o workflow `Release`, que compila os arquivos e publica um **pre-release** no GitHub.
3. **Testar o release publicado.** Quando o `Release` termina com sucesso, o `Release smoke` instala o tokred a partir dele em sistemas reais (veja Smoke test do release, abaixo).
4. **Promover.** Com o smoke test verde, `scripts/release.sh promote 0.2.0-rc.1` define a versão final `0.2.0` sobre o mesmo commit, faz o commit, cria a tag `v0.2.0` e dá push. Os mesmos dois workflows rodam de novo para o release final.

Não anuncie uma versão (uma mensagem, um link no README) enquanto algum job do `Release smoke` da tag dela estiver vermelho.

## Cortando uma versão

Execute o script na `main`, em um checkout limpo, ou use **Actions, Cut a release, Run workflow** com a ação (`rc` ou `promote`), a versão do RC e, opcionalmente, `dry_run`.

```sh
scripts/release.sh rc 0.2.0-rc.1
```

```sh
scripts/release.sh promote 0.2.0-rc.1
```

O script precisa de `git`, `python3` e do `gh` autenticado. As duas ações exigem a árvore de trabalho limpa e a `main` igual à `origin/main`, e recusam caso contrário.

| Ação | Verifica antes | Depois |
|---|---|---|
| `rc <X.Y.Z-rc.N>` | O CI passou (uma execução de `push`) no commit; a tag não existe; os dois changelogs têm entradas em `Unreleased` / `Não lançado` | Define a versão do workspace e atualiza o `Cargo.lock`, move as entradas do changelog para `[X.Y.Z-rc.N] - <data>` em `CHANGELOG.md` e `CHANGELOG.pt-BR.md`, faz o commit `chore: release X.Y.Z-rc.N`, cria a tag e dá push na `main` e na tag |
| `promote <X.Y.Z-rc.N>` | A `main` ainda está no commit do RC; o RC está publicado como pre-release; o CI passou nesse commit; o `Release smoke` do RC teve sucesso | Define a versão `X.Y.Z`, transforma o título do RC no changelog em `[X.Y.Z] - <data>`, faz o commit, cria a tag e dá push |

Se uma verificação falhar depois que os arquivos foram editados, o script os restaura. Se a `main` andou depois do RC, corte um novo RC com o que mudou, teste-o e promova esse.

**Dry run.** `--dry-run` (`-n`) executa as verificações e edita os arquivos, mas não faz commit, tag nem push; desfaça com `git checkout -- .`. O workflow tem a mesma entrada `dry_run`. Para um ensaio de verdade, um RC já é um: ele é publicado como pre-release, que nunca é o "latest", então ninguém o recebe por acidente.

### O segredo `RELEASE_TOKEN`

O `Cut a release` precisa de um segredo do repositório chamado `RELEASE_TOKEN`: um personal access token fine-grained com `Contents: read and write` neste repositório. O `GITHUB_TOKEN` padrão não serve, porque uma tag enviada com ele não dispara outros workflows, então o release nunca seria compilado. O workflow falha com uma mensagem quando o segredo está ausente.

## O que um release contém

O workflow `Release` é gerado pelo `dist` e começa em uma tag com cara de versão (`v0.2.0`, `v0.2.0-rc.1`). Ele compila cinco alvos e envia estes arquivos ao release do GitHub:

| Arquivo | O que é |
|---|---|
| `tokred-<target>.tar.gz` / `.zip` | O binário com o README, as licenças e o changelog. Alvos: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl`, `x86_64-pc-windows-msvc` |
| `tokred-<target>.tar.gz.sha256` / `.zip.sha256` | O SHA-256 do arquivo; os scripts de instalação e o `tokred update` o verificam |
| `install.sh`, `install.ps1`, `uninstall.sh`, `uninstall.ps1` | Os scripts que os usuários executam, copiados de `install/` |
| `source.tar.gz` (e o seu `.sha256`), `sha256.sum` | Acrescentados pelo `dist` |

As notas do release vêm da seção correspondente do `CHANGELOG.md`. O workflow `Release` só precisa do `GITHUB_TOKEN` do próprio repositório; nenhum gerenciador de pacotes ou registro está envolvido.

Uma versão com sufixo (`-rc.1`) é publicada como **pre-release**. `releases/latest/download/` sempre aponta para o release estável mais novo, então quem executa o comando de instalação simples nunca recebe um RC. Para instalar uma versão específica, informe a versão:

**macOS e Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/download/v0.2.0-rc.1/install.sh | sh -s -- --version 0.2.0-rc.1
```

**Windows (PowerShell)**

```powershell
$env:TOKRED_VERSION = "0.2.0-rc.1"; irm https://github.com/sostenes198/tokred/releases/download/v0.2.0-rc.1/install.ps1 | iex
```

## Smoke test do release

Os testes de [development](development.pt-BR.md) rodam contra código compilado da árvore de trabalho. O smoke test prova o que o usuário recebe: ele instala o tokred a partir do release **publicado** (o `install.sh` ou `install.ps1` do próprio release, que baixam o binário real) e confere cada recurso contra estado real, não "rodou".

Ele roda sozinho quando o `Release` termina com sucesso para uma tag `v*`, em runners Ubuntu, macOS (Apple Silicon e Intel) e Windows e em containers Alpine, Debian e Fedora (o Alpine não tem glibc, então uma execução verde prova o binário musl estático). Você também pode executá-lo para qualquer tag em **Actions, Release smoke, Run workflow**, ou manualmente:

```sh
sh tests/release/smoke.sh v0.2.0-rc.1
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tests\release\smoke.ps1 -Tag v0.2.0-rc.1 -AllowRealProfile
```

Os arquivos são `tests/release/smoke.sh` (`sh` POSIX), `smoke.ps1` (Windows PowerShell 5.1 e 7), `check.py` (asserções de JSON, TOML e SQLite) e `fake_upstream.py` (uma API Anthropic falsa que faz streaming devagar). Precisa de Python 3.7+ e `curl`. Tudo roda em um diretório descartável (`HOME`, `TOKRED_HOME`, `TOKRED_DATA_DIR`, `CLAUDE_CONFIG_DIR`), partindo de um `settings.json` e de perfis de shell do próprio usuário. No Windows o `PATH` de usuário e os perfis do PowerShell não podem ficar em sandbox: o script faz backup e restaura, e fora do CI pede `-AllowRealProfile`.

### O que ele confere

| Etapa | Prova |
|---|---|
| 0. Download | O `install.sh` e o `uninstall.sh` do release são baixados da tag e se verifica que existem |
| 1. Instalação | `tokred --version` é igual à tag; o binário está no lugar; um bloco marcado em cada perfil de shell com as linhas do usuário intactas; os seis hooks e agentes estão instalados ao lado das chaves e do hook do próprio usuário; o proxy responde no endpoint de saúde e a `ANTHROPIC_BASE_URL` aponta para ele |
| 2. O wrapper `claude` | Em `sh`, `bash`, `dash` e `zsh` novos: `claude` é uma função, o `tokred initialize` realmente rodou, os argumentos chegam intactos e, sem o tokred no `PATH`, o `claude` ainda executa |
| 3. Tráfego pelo proxy | Contra o upstream falso: requisições de loop principal e de subagente em streaming voltam como os bytes do falso; o link do dashboard funciona uma vez; a API de sessões e o detalhe de volta estão certos; `/metrics` é HTML; um `Host` errado é 421; `stats --json` tem a seção do proxy; desligar a gravação de conteúdo é aplicado |
| 4. `unconfigure` e `configure` | O unconfigure remove hooks e agentes não editados e mantém as chaves do usuário e a URL do proxy; o configure traz tudo de volta |
| 5. Chave do proxy | `proxy disable` remove só a URL do tokred e deixa a de terceiros; o wrapper não religa; `proxy enable` mantém a porta salva |
| 6. Update | Com só pre-releases publicados, `tokred update` sai com 1 e diz que não existe release estável. Quando existe um release estável, verifica que o comando sai com 0 e informa que está atualizado ou que há versão mais nova; se o binário foi substituído corretamente não é verificado |
| 7. Desinstalação, duas vezes | Com o binário instalado e, depois de reinstalar, com o script de desinstalação do release: nada sobra, o `settings.json` é igual ao documento inicial e os perfis ficam byte a byte como o usuário tinha. Duas coisas ficam por desenho e são impressas como notas: `settings.json.tokred.bak` e uma pasta `agents/` vazia (um agente que o usuário editou também é mantido) |
| 8. Limpeza | Sempre (um trap): o proxy e o upstream falso são parados. Depois de uma falha, as últimas 30 linhas de cada log são impressas |

### Lendo uma falha

- A verificação que falhou imprime `not ok - [<etapa>] <o esperado>: <o encontrado>` e o código de saída é 1. Depois dela vêm as últimas 30 linhas de cada log. No CI os logs são o artefato `smoke-logs-*` do job que falhou; localmente, `SMOKE_KEEP=1` mantém o sandbox inteiro.
- Uma falha na etapa 1 ou 7 em **uma única plataforma** é problema de empacotamento ou do instalador. Uma falha nas etapas 3 a 5 em **todas as plataformas** é bug do produto: reproduza com os comandos da etapa em um sandbox ([development](development.pt-BR.md)).
- Linhas `skip - ...` não são falhas: um shell que não está instalado, ou o limite de taxa da API do GitHub na etapa 6.
- Um job vermelho é um achado sobre os artefatos publicados. Corrija na `main` e corte o próximo RC; não promova.

Opções, documentadas no topo de cada script: `SMOKE_TAG`, `SMOKE_REPO`, `SMOKE_PORT`, `SMOKE_DIR`, `SMOKE_KEEP=1`, `SMOKE_LOG_DIR`, `SMOKE_CONTINUE=1` (conta as falhas e segue; um modo de diagnóstico, o código de saída continua sendo 1), `SMOKE_SKIP_UPDATE=1`, `SMOKE_PYTHON`, `GITHUB_TOKEN` (só para a consulta do próprio script à API do GitHub).

### O que ele não consegue cobrir

O próprio Claude Code com um login real: as requisições têm o formato das do Claude Code e vão para uma API falsa, então tráfego de assinatura, saída real do modelo e a inicialização do Claude Code não são testados. Ele também não testa um binário que ainda não foi publicado (os testes do instalador em [development](development.pt-BR.md) cobrem os scripts) nem o ramo do `tokred update` que substitui o binário.

## Quando algo dá errado

- **Um job de compilação falha.** Corrija na `main` e corte um novo RC. Nada é publicado se a falha foi antes do job `host`; para reaproveitar uma tag que nunca foi publicada, apague-a (`git push origin :refs/tags/v0.2.0-rc.1` e `git tag -d v0.2.0-rc.1`).
- **Um release ruim saiu.** Publique uma nova versão de correção; o `latest` passa a apontar para ela. Para retirar o ruim, apague o release no GitHub (a tag pode ficar ou sair).
- **Um script de instalação tem um bug.** Corrija na `main` e corte um novo release; a cópia de `install/` na `main` entrega a correção na hora para quem usa essa URL.

## O que o `dist-workspace.toml` controla

Ele configura o `dist` (a versão do `dist` usada no CI fica fixada nele): os cinco alvos, os formatos de arquivo (`.tar.gz`, `.zip`), os checksums `.sha256`, nenhum instalador próprio (os scripts de `install/` são usados) e os `extra-artifacts` que enviam os quatro scripts de instalação junto dos arquivos.

Depois de alterá-lo, execute `dist generate` e faça o commit do `.github/workflows/release.yml` alterado; não edite esse arquivo à mão. `dist plan` mostra o que um release conteria, e `dist build --artifacts=local` compila os arquivos para o alvo da sua máquina sem publicar nada. Instale o `dist` com `cargo install cargo-dist --locked`.

`allow-dirty = ["ci"]` está definido para que as atualizações de versão de actions feitas pelo Dependabot dentro do workflow gerado não façam o `dist plan` falhar com "out of date contents".
