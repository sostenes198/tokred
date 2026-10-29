[English](installation.md) | **Português**

# Instalação

Instalar e configurar são **dois passos separados**. Nenhum canal de instalação mexe na pasta do Claude Code; a integração só acontece quando você roda `tokred configure`.

```sh
brew install sostenes198/tap/tokred   # 1. instala a CLI
tokred configure                      # 2. integra com o Claude Code
```

## Canais

| SO | Comando | Como é publicado |
|---|---|---|
| macOS / Linux | `brew install sostenes198/tap/tokred` | Fórmula no tap `sostenes198/homebrew-tap`, gerada pelo `dist` a cada release estável |
| Windows | `winget install tokred` | Manifest enviado ao `winget-pkgs` pelo CI a cada release |
| Windows | `scoop bucket add sostenes198 https://github.com/sostenes198/scoop-bucket` e depois `scoop install sostenes198/tokred` | Manifest no bucket `sostenes198/scoop-bucket`, atualizado pelo CI |
| Qualquer | script shell (`curl … \| sh`) ou PowerShell (`irm … \| iex`) | Gerados pelo `dist` |
| Qualquer | `cargo install tokred` | crates.io |

As releases são construídas pelo [`dist`](https://opensource.axo.dev/cargo-dist/) para macOS, Linux (musl) e Windows em x86_64 e aarch64, cada arquivo com um checksum `.sha256`. Antes de a primeira release ser publicada, compile do código-fonte:

```sh
cargo install --path crates/tokred-cli
```

## Opcional: o proxy local

O [proxy](proxy.pt-BR.md) é controlado por uma chave única, `[proxy] enabled` no `tokred.toml` (o `tokred configure` a liga em uma instalação nova; o `tokred proxy disable` a desliga). Enquanto está ligada, o `tokred initialize` (o que o wrapper de shell do `claude` executa antes de toda abertura) sobe o proxy quando preciso e mantém o `env.ANTHROPIC_BASE_URL` do `settings.json` do Claude Code apontando para ele; o `tokred unconfigure` remove essa URL de novo. O seu próprio gateway em `ANTHROPIC_BASE_URL` nunca é sobrescrito.

```sh
tokred claude                     # inicia o proxy se preciso e executa o Claude Code por ele, só nesta execução
tokred proxy service install      # opcional: inicia o proxy no login, como serviço de usuário
```

`tokred compression on` é o único comando que edita a configuração de usuário do Claude Code (`~/.claude.json`, com backup em `.tokred.bak`), e só quando você o executa.

## Atualização

O `tokred update` detecta como o tokred foi instalado pelo path do executável e faz a coisa certa:

| Método | Path contém | O que `tokred update` faz |
|---|---|---|
| Homebrew | `Cellar` ou `linuxbrew` | Mostra `brew upgrade tokred` |
| winget | `WinGet\Packages` ou `WinGet\Links` | Mostra `winget upgrade tokred` |
| scoop | `scoop\apps` ou `scoop\shims` | Mostra `scoop update tokred` |
| cargo | `.cargo/bin` | Mostra `cargo install tokred --force` |
| Script | qualquer outro | Faz o self-update (download, checksum, troca atômica) |

Depois que o binário muda de versão, o primeiro comando da CLI reaplica a configuração de forma não interativa. Hooks nunca fazem isso; o `tokred doctor` mostra quando há reconfiguração pendente.

## Verificação de versão

Uma vez por dia, só a partir de comandos da CLI (nunca de um hook), o tokred consulta o GitHub pela última release com timeout de 2 s e, se houver versão mais nova, imprime um aviso de uma linha no **stderr**, para não afetar a saída `--json`. Desligue com `[update] mode = "off"` ou `TOKRED_NO_UPDATE_CHECK=1`; `TOKRED_OFFLINE=1` desliga toda chamada de rede (verificação de versão e sincronização de preços).

## Publicando uma release (mantenedores)

Envie uma tag `vX.Y.Z`. O `release.yml` gerado constrói os arquivos e instaladores e publica a fórmula do Homebrew; depois o `publish-package-managers.yml` atualiza o bucket do Scoop, abre o pull request do winget, adiciona o aviso de `tokred configure` à fórmula e publica os crates no crates.io. Estes secrets precisam existir no repositório: `HOMEBREW_TAP_TOKEN`, `SCOOP_BUCKET_TOKEN`, `WINGET_TOKEN`, `CARGO_REGISTRY_TOKEN`. O pacote do winget precisa ter o primeiro manifest enviado à mão uma vez. Passo a passo: [release](release.pt-BR.md).

## Desinstalação

```sh
tokred proxy service uninstall   # só se você instalou o serviço do proxy
tokred compression off            # só se você ligou a compressão
tokred unconfigure --purge        # remove hooks, agentes e a pasta de dados
brew uninstall tokred             # ou: winget uninstall tokred / scoop uninstall tokred / cargo uninstall tokred
```

O `tokred unconfigure` remove só o que o tokred adicionou à pasta do Claude Code. O binário em si é removido pelo gerenciador de pacotes.
