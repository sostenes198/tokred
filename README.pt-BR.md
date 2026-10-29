[English](README.md) | **Português**

# tokred

[![CI](https://github.com/sostenes198/tokred/actions/workflows/ci.yml/badge.svg)](https://github.com/sostenes198/tokred/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/sostenes198/tokred?include_prereleases)](https://github.com/sostenes198/tokred/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#contribuição-segurança-licença)

Reduza o custo e o uso de tokens do Claude Code: roteamento de modelos, proteções de contexto e métricas locais.

## O que é

O tokred é uma ferramenta de linha de comando local para macOS, Linux e Windows. Ele funciona em duas camadas:

1. **Hooks, subagentes e métricas.** Seis subagentes fixados no modelo certo para cada tipo de tarefa, hooks que reforçam essa delegação e bloqueiam leituras inúteis de contexto, e um relatório de tokens, custo e economia.
2. **Um proxy local opcional** entre o Claude Code e a API da Anthropic. Ele mede cada requisição com exatidão, roteia para um modelo mais barato apenas as requisições de subagentes e auxiliares, aconselha você sobre o loop principal (nunca troca o modelo do loop principal), mantém resultados grandes de ferramentas fora do contexto sem perdê-los e serve um dashboard ao vivo.

Tudo roda na sua máquina, sem conta e sem telemetria. Os números de economia são estimativas, não garantias.

## Requisitos

- [Claude Code](https://code.claude.com)
- macOS, Linux ou Windows. Você não precisa de Rust para usar o tokred.

## Instalação

**macOS e Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

O instalador:

- baixa a release da sua plataforma e verifica o checksum SHA-256;
- instala o binário em `~/.tokred/bin` (Windows: `%LOCALAPPDATA%\tokred\bin`) e o adiciona ao seu `PATH`, sem `sudo`;
- executa `tokred configure` (hooks e agentes) e `tokred proxy enable` (pergunta a porta do proxy e se o dashboard pode gravar o conteúdo das conversas; `--no-proxy` pula esse passo).

Ele também define um wrapper `claude` no perfil do seu shell, que mantém as configurações do Claude Code alinhadas com a chave do proxy antes de cada execução; `command claude` roda o binário real.

Para instalar uma versão específica, passe `--version X` (PowerShell: `-Version X`, ou `TOKRED_VERSION=X`). Pré-releases só são instaladas com uma versão explícita.

> Até a primeira release estável ser publicada, as URLs `latest` acima não existem. Use `--version` com uma tag de release, ou os scripts da `main`: `https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.sh` e `https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.ps1`.

Todas as opções, o wrapper e a resolução de problemas: [instalação](docs/installation.pt-BR.md).

## Primeiros passos

```sh
tokred doctor     # confere o que está instalado
tokred stats      # tokens, custo e economia no terminal
tokred metrics    # o mesmo, mais um relatório em HTML
tokred dashboard  # visão ao vivo das suas sessões (precisa do proxy)
tokred claude     # roda o Claude Code pelo proxy, só nesta execução
```

## Comandos

| Comando | O que faz |
|---|---|
| `tokred configure` | Adiciona os hooks e agentes do tokred ao Claude Code e cria o banco de dados e a configuração |
| `tokred doctor` | Mostra o que está instalado e o que precisa de atenção |
| `tokred stats` | Tokens, custo, economia e projeção |
| `tokred metrics` | O mesmo, mais um relatório em HTML |
| `tokred dashboard` | Abre o dashboard ao vivo no navegador |
| `tokred proxy enable` / `disable` / `status` | Liga ou desliga o proxy, ou mostra o que ele está fazendo |
| `tokred update` | Atualiza o tokred (quando o script o instalou) |
| `tokred unconfigure` | Remove apenas os hooks e agentes do tokred |
| `tokred uninstall` | Remove o tokred por completo |

Todos os comandos e opções: [comandos](docs/commands.pt-BR.md).

## Como funciona

O Claude Code executa um hook do tokred antes ou depois de certos eventos e passa JSON pela entrada padrão; o tokred responde com uma decisão ou uma dica de uma linha. Os hooks são fail-open: diante de qualquer erro, pânico ou timeout eles saem com código 0 e sem saída, então um bug no tokred não consegue bloquear o Claude Code. O proxy, quando habilitado, encaminha cada requisição à API da Anthropic e registra o que passou. Veja [arquitetura](docs/architecture.pt-BR.md).

## Privacidade

- **Local.** Métricas e configurações ficam na sua própria pasta de dados. O proxy escuta apenas em `127.0.0.1`, nunca registra credenciais e encaminha requisições somente à API da Anthropic.
- **O conteúdo das mensagens** só é guardado se você ativar `[proxy] log_bodies` (o instalador pergunta; sem terminal, o padrão é desligado). Detalhes: [dashboard](docs/dashboard.pt-BR.md) e [proxy](docs/proxy.pt-BR.md).
- **Rede.** Além do proxy, o tokred acessa o GitHub (verificação de versão, `tokred update`) e busca uma tabela de preços, somente a partir de comandos da CLI e nunca de hooks. Desligue com `[update] mode = "off"`, `[pricing] auto_sync = false`, ou `TOKRED_OFFLINE=1` para todos. Veja [configuração](docs/configuration.pt-BR.md).

## Documentação

O índice completo está em [docs/README.pt-BR.md](docs/README.pt-BR.md): instalação, comandos, configuração, hooks e agentes, métricas, proxy, roteamento, dashboard e os guias para quem contribui.

## Desinstalação

**macOS e Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/uninstall.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/sostenes198/tokred/releases/latest/download/uninstall.ps1 | iex
```

Ela remove tudo sem perguntar: o proxy e seu serviço, os hooks e agentes, a pasta de dados do tokred, o binário, a entrada no `PATH` e o wrapper `claude`.

## Contribuição, segurança, licença

- Contribuições são bem-vindas: veja [CONTRIBUTING.pt-BR.md](CONTRIBUTING.pt-BR.md) e o [código de conduta](CODE_OF_CONDUCT.pt-BR.md).
- Reporte vulnerabilidades em particular: veja [SECURITY.pt-BR.md](SECURITY.pt-BR.md).
- Licenciado sob a [Apache License 2.0](LICENSE-APACHE) ou a [licença MIT](LICENSE-MIT), à sua escolha.
