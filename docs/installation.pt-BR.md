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

> **Status:** o pipeline de release (`dist`, tap, bucket, winget) faz parte da fase F1. Até a primeira release, compile do código-fonte:
>
> ```sh
> cargo install --path crates/tokred-cli
> ```

## Atualização

O `tokred update` (fase F1) detecta como o tokred foi instalado pelo path do executável e faz a coisa certa:

| Método | Path contém | O que `tokred update` faz |
|---|---|---|
| Homebrew | `Cellar` ou `linuxbrew` | Mostra `brew upgrade tokred` |
| winget | `WinGet\Packages` ou `WinGet\Links` | Mostra `winget upgrade tokred` |
| scoop | `scoop\apps` ou `scoop\shims` | Mostra `scoop update tokred` |
| cargo | `.cargo/bin` | Mostra `cargo install tokred --force` |
| Script | qualquer outro | Faz o self-update (download, checksum, troca atômica) |

Depois que o binário muda de versão, o primeiro comando da CLI reaplica a configuração de forma não interativa. Hooks nunca fazem isso; o `tokred doctor` mostra quando há reconfiguração pendente.

## Verificação de versão

Uma vez por dia, só a partir de comandos da CLI (nunca de um hook), o tokred consulta o GitHub pela última release com timeout de 2 s e, se houver versão mais nova, imprime um aviso de uma linha no **stderr**, para não afetar a saída `--json`. Desligue com `[update] mode = "off"` ou `TOKRED_NO_UPDATE_CHECK=1`. (Fase F1.)

## Desinstalação

```sh
tokred unconfigure --purge   # remove hooks, agentes e a pasta de dados
brew uninstall tokred        # ou: winget uninstall tokred / scoop uninstall tokred / cargo uninstall tokred
```

O `tokred unconfigure` remove só o que o tokred adicionou à pasta do Claude Code. O binário em si é removido pelo gerenciador de pacotes.
