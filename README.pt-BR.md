[English](README.md) | **Português**

# tokred

[![CI](https://github.com/sostenes198/tokred/actions/workflows/ci.yml/badge.svg)](https://github.com/sostenes198/tokred/actions/workflows/ci.yml)
[![Licença](https://img.shields.io/badge/licen%C3%A7a-MIT%20OR%20Apache--2.0-blue)](#licença)

*Redutor de tokens e custo para o Claude Code — roteamento de modelo, guardas de contexto e métricas locais. Funciona junto com o rtk.*

## O que é

tokred é uma CLI em Rust, local e multiplataforma, que reduz o custo e o consumo de tokens do Claude Code. Ela instala subagentes com o modelo certo para cada tipo de tarefa (opus para arquitetura e planejamento, sonnet para desenvolvimento, haiku para busca e resumo), hooks que reforçam essa delegação e bloqueiam leituras inúteis de contexto, e um relatório de métricas com tokens, custo, economia e a latência que o próprio tokred acrescenta.

Tudo roda na sua máquina, com um SQLite próprio e sem serviço externo. Foi pensada para rodar junto com o [rtk](https://github.com/rtk-ai/rtk) (rust token killer), que comprime a saída de comandos; o tokred cuida do resto. Os dois convivem: o rtk fica com o matcher `Bash` e o tokred nunca registra um.

> **Status:** as fases F0 a F5 estão implementadas: configure/unconfigure/doctor, os seis hooks, métricas, preços, audit, update e o pipeline de release (veja [a spec](docs/spec.pt-BR.md) e [o changelog](CHANGELOG.pt-BR.md)). Há também um proxy local opcional implementado, como camada aditiva (medição, roteamento de requisições de subagentes e auxiliares, conselhos para o loop principal, compressão reversível): veja [proxy](docs/proxy.pt-BR.md) e [roteamento](docs/routing.pt-BR.md). A [spec final](docs/spec.pt-BR.md) diz o que está feito e o que ainda falta.

## Funcionalidades

- **Instalação não destrutiva:** `tokred configure` faz o merge dos hooks no `settings.json` sem mexer em mais nada, e `tokred unconfigure` remove exatamente o que foi adicionado → [comandos](docs/commands.pt-BR.md)
- **Subagentes por modelo:** seis subagentes com modelo fixo e formato de retorno curto → [agentes](docs/agents.pt-BR.md)
- **Hooks que nunca atrapalham:** fail-open, modos `off | warn | enforce`; roteiam o trabalho para o modelo certo e bloqueiam leituras inúteis → [hooks](docs/hooks.pt-BR.md)
- **Números no seu terminal:** `tokred stats` mostra tokens, custo, economia, projeção e a latência que o tokred acrescenta, num painel colorido; `tokred metrics` também gera um relatório HTML offline → [métricas](docs/metrics.pt-BR.md)
- **Proxy local opcional:** fica entre o Claude Code e a API para medir cada requisição com exatidão, rotear requisições de subagentes e auxiliares para um modelo mais barato, aconselhar você sobre o loop principal e manter resultados grandes de ferramentas fora do contexto sem perdê-los. Desligado até você rodar `tokred claude`; o modelo do loop principal nunca é alterado → [proxy](docs/proxy.pt-BR.md), [roteamento](docs/routing.pt-BR.md)
- **Audit:** quão pesado é seu contexto fixo e quais servidores MCP você nunca usa → [comandos](docs/commands.pt-BR.md)
- **Store local:** SQLite com migrations versionadas e cache em camadas → [cache](docs/cache.pt-BR.md)

## Instalação

Instalar o binário nunca mexe no Claude Code; isso é um passo separado (`tokred configure`).

| SO | Comando |
|---|---|
| macOS / Linux | `brew install sostenes198/tap/tokred` |
| Windows | `winget install tokred` ou `scoop bucket add sostenes198 https://github.com/sostenes198/scoop-bucket` e depois `scoop install sostenes198/tokred` |
| Qualquer | `cargo install tokred` ou o instalador shell / PowerShell da página de releases |

Até a primeira release ser publicada, compile do código-fonte (`cargo install --path crates/tokred-cli`). Detalhes em [instalação](docs/installation.pt-BR.md).

## Início rápido

```sh
tokred configure    # integra com o Claude Code (mostra um plano e pede confirmação)
tokred doctor       # confere o que está instalado
tokred stats        # tokens, custo e economia, direto no terminal
tokred metrics      # o mesmo, mais o relatório HTML
```

## Comandos

| Comando | O que faz |
|---|---|
| `tokred configure [--claude-dir <path>] [--yes] [--force]` | Faz o merge dos hooks no `settings.json`, instala os agentes, cria o banco e a config |
| `tokred unconfigure [--purge]` | Remove só as partes do tokred; `--purge` também apaga a pasta de dados |
| `tokred doctor [--json] [--bench]` | Mostra o que está instalado e o que precisa de atenção; mede a latência dos hooks |
| `tokred stats [--since 7d] [--by model\|day\|project]` | Painel de tokens, custo, economia e projeção |
| `tokred stats latency` | Tempo que os hooks acrescentaram ao Claude Code |
| `tokred metrics [--since 30d] [--no-open]` | O painel mais um relatório HTML |
| `tokred audit [--project <path>]` | Peso do contexto fixo e MCPs sem uso |
| `tokred update` | Atualiza conforme o método de instalação |
| `tokred pricing sync\|show`, `tokred cache stats\|prune` | Tabela de preços e cache |
| `tokred claude [args...]` | Executa o Claude Code pelo proxy opcional, só nesta execução |
| `tokred proxy start\|stop\|status`, `tokred proxy service install\|uninstall` | Executa e inspeciona o proxy; inicia no login |
| `tokred routing status\|on\|off\|explain`, `tokred compression on\|off\|status` | Roteamento de modelo e compressão reversível (ambos em `observe` por padrão) |
| `tokred suggest`, `tokred statusline` | Sugestões de modelo mais barato para o loop principal |

Referência completa em [comandos](docs/commands.pt-BR.md).

## Como funciona

O Claude Code executa um hook do tokred antes ou depois de certos eventos e envia JSON no stdin; o tokred responde com uma decisão ou uma dica de uma linha. Todo hook é fail-open: em qualquer erro, panic ou timeout ele sai com código 0 e sem saída, então um bug do tokred nunca trava o Claude Code. Veja [arquitetura](docs/architecture.pt-BR.md) e [hooks](docs/hooks.pt-BR.md).

## Configuração

Um único `tokred.toml` na pasta de dados, com merge (nunca substituição) a cada atualização. Veja [configuração](docs/configuration.pt-BR.md).

## Privacidade

Tudo fica na sua máquina. As únicas chamadas de rede (a sincronização diária de preços, a verificação de versão e o `update`) saem de comandos da CLI, nunca de hooks, e podem ser desligadas: `[pricing] auto_sync = false`, `[update] mode = "off"`, `TOKRED_NO_UPDATE_CHECK=1`, ou `TOKRED_OFFLINE=1` para todas. O proxy opcional escuta só em `127.0.0.1`, nunca registra credenciais nem o conteúdo das mensagens e encaminha requisições apenas para a API da Anthropic.

## Desinstalação

```sh
tokred unconfigure --purge   # remove hooks, agentes e os dados do tokred
brew uninstall tokred        # ou: winget uninstall tokred / scoop uninstall tokred / cargo uninstall tokred
```

## Contribuindo

Veja [CONTRIBUTING.pt-BR.md](CONTRIBUTING.pt-BR.md). Para rodar e testar localmente, veja [testes](docs/testing.pt-BR.md); para publicar uma release (Homebrew, Scoop, winget, crates.io), veja [release](docs/release.pt-BR.md).

## Licença

Licenciado sob a [Apache License 2.0](LICENSE-APACHE) ou a [licença MIT](LICENSE-MIT), à sua escolha.
