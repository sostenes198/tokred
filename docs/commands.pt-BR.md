[English](commands.md) | **Português**

# Comandos

Todo comando que imprime um relatório aceita `--json` para saída legível por máquina. As tabelas saem coloridas quando o stdout é um terminal; use `NO_COLOR=1` para desligar as cores (`CLICOLOR_FORCE=1` as força).

| Comando | O que faz |
|---|---|
| `tokred configure` | Integra com o Claude Code |
| `tokred update` | Atualiza conforme o método de instalação |
| `tokred unconfigure` | Remove as partes do tokred do Claude Code |
| `tokred doctor` | Mostra o que está instalado e o que precisa de atenção |
| `tokred stats` | Tokens, custo, economia e projeção, no terminal |
| `tokred stats latency` | Tempo que os hooks acrescentaram ao Claude Code |
| `tokred metrics` | Os mesmos números mais um relatório HTML |
| `tokred audit` | Peso do contexto fixo e MCPs sem uso |
| `tokred pricing sync` / `show` | Atualiza ou mostra a tabela de preços |
| `tokred cache stats` / `prune` | Estado do cache em camadas; remove expirados |
| `tokred hook <evento>` | Interno: chamado pelo Claude Code |

A pasta do Claude Code pode ser dada com `--claude-dir <path>` em todo comando que precisa dela.

## `tokred configure [--claude-dir <path>] [--yes] [--force]`

Integra o tokred com o Claude Code.

1. Encontra a pasta do Claude Code, nesta ordem: `--claude-dir`, o valor salvo no `tokred.toml` (`[claude] dir`), `$CLAUDE_CONFIG_DIR`, `~/.claude` (`%USERPROFILE%\.claude` no Windows). Uma pasta é válida quando existe e contém `settings.json` ou `projects/`. Um `--claude-dir` explícito que não seja válido é erro. Se nada for encontrado, um terminal interativo é perguntado pelo path; senão o comando falha e sugere `--claude-dir`.
2. Detecta o rtk e informa que os dois vão coexistir.
3. Mostra o **plano**: hooks a adicionar (`+`), atualizar (`~`) ou manter (`=`), agentes a criar, atualizar ou deixar como estão, e o backup que será feito. Pede confirmação; `--yes` pula a pergunta (obrigatório quando o stdin não é um terminal).
4. Aplica: cria a pasta de dados, o banco e o `tokred.toml`, faz o merge do `settings.json` (backup em `settings.json.tokred.bak`, mantendo só o último) e instala os agentes.

Rodar de novo é seguro: o resultado é o mesmo. Se o `settings.json` não for um JSON válido, o comando aborta e **não escreve nada**. `--force` sobrescreve os arquivos de agente mesmo que você os tenha editado (sem ele, sua versão é mantida e a nova vai para `tokred-x.md.new`).

## `tokred update [--claude-dir <path>]`

Consulta a última release e age conforme o método de instalação (detectado pelo path do executável):

| Instalado com | `tokred update` |
|---|---|
| Homebrew, winget, scoop, cargo | Mostra o comando a rodar (`brew upgrade tokred`, `winget upgrade tokred`, `scoop update tokred`, `cargo install tokred --force`) |
| Script de instalação | Baixa o arquivo para esta plataforma, **verifica o SHA-256**, troca o binário de forma atômica e reaplica a configuração com o binário novo |

Depois que o binário muda de versão, o primeiro comando da CLI que você rodar reaplica a configuração em silêncio (hooks nunca fazem isso), então um upgrade pelo gerenciador de pacotes não exige passo extra. Precisa de rede; se recusa a rodar com `TOKRED_OFFLINE` definido.

## `tokred unconfigure [--claude-dir <path>] [--purge]`

Remove os hooks do tokred do `settings.json` e os arquivos de agente que o tokred instalou e que você não editou. Agentes editados são mantidos e informados. Arquivos sem o prefixo `tokred-` nunca são tocados, e o que o tokred não entende no `settings.json` fica como está. `--purge` também apaga a pasta de dados; ele se recusa a fazer isso se a pasta não contiver `tokred.db` ou `tokred.toml`.

## `tokred doctor [--claude-dir <path>] [--json] [--bench]`

Mostra a versão do tokred, o path do binário, as pastas do Claude Code e de dados, a versão do schema do banco, o estado da config, a detecção do rtk, uma tabela de hooks (`ok`, `missing`, `outdated`, ou path de binário que não existe mais), uma tabela de agentes (`current`, `outdated`, `edited by you`, `missing`, `.new` pendente) e uma lista de problemas e observações. Com `--bench` também executa cada hook muitas vezes com entrada realista e mede a latência real por fora (do início do processo até o fim); o resultado é salvo e usado pelo `stats latency`.

## `tokred stats [--since 7d] [--by model|day|project] [--json]`

O painel de custo, impresso no terminal: totais (mensagens, tokens, custo real), quanto os mesmos tokens custariam se tudo rodasse no opus, a economia do roteamento e das guardas (**uma estimativa**, rotulada como tal), a taxa de acerto do cache de prompt, a projeção mensal, um medidor de eficiência e uma tabela numerada com coluna de participação e barra de impacto. Modelos precificados por família aparecem sinalizados. Quando há dados anteriores ao primeiro `configure`, compara o custo por sessão antes e depois. Os transcripts são ingeridos de forma incremental antes, então fica rápido depois da primeira vez. `--since` aceita horas, dias e semanas (`12h`, `7d`, `2w`).

## `tokred stats latency [--since 7d] [--by hook|day|session] [--json]`

Quanto tempo os hooks do tokred acrescentaram ao Claude Code: execuções, p50/p95/p99/máximo por hook, tempo total acrescentado (uma estimativa: inclui o custo de iniciar o processo medido pelo `doctor --bench`), média por prompt, resultados (ok, erro, panic, timeout guard), um alerta quando o p95 de um hook passa do orçamento de 50 ms e a comparação com o que as guardas economizaram no mesmo período.

## `tokred metrics [--since 30d] [--no-open]`

Imprime o mesmo painel do `stats` e grava o relatório HTML autocontido em `<dados>/reports/metrics.html` (sempre o mesmo arquivo, sobrescrito de forma atômica), e o abre no navegador a menos que se passe `--no-open`. O relatório funciona offline: sem rede, sem fontes nem scripts externos.

## `tokred audit [--project <path>] [--json]`

Estima (caracteres / 4) o peso do contexto que o Claude Code carrega antes de você digitar: arquivos `CLAUDE.md` com seus imports `@`, rules, skills e agentes; e lista os servidores MCP sem uso nos últimos 30 dias. O projeto é a pasta atual por padrão. O peso exato dos schemas das ferramentas MCP não pode ser calculado localmente, então só o uso é mostrado.

## `tokred pricing sync` / `tokred pricing show [--json]`

`sync` atualiza a tabela de preços a partir da lista pública do LiteLLM; se ela não puder ser alcançada, o snapshot embutido continua em uso. `show` imprime a tabela (USD por milhão de tokens). Os preços também são sincronizados automaticamente, no máximo uma vez por dia, pelos comandos acima.

## `tokred cache stats [--json]` / `tokred cache prune`

`stats` mostra entradas e acertos por camada (L1/L2/L3); `prune` remove os expirados. Veja [cache](cache.pt-BR.md).

## `tokred hook <evento>` (interno)

Chamado pelo Claude Code, nunca à mão (exceto para testes, veja [desenvolvimento](development.pt-BR.md)). Lê JSON do stdin e escreve JSON no stdout. Eventos: `route`, `enforce`, `guard-read`, `guard-search`, `session-check`, `session-start`. **Sempre sai com código 0**, mesmo para evento desconhecido ou entrada inválida. Veja [hooks](hooks.pt-BR.md).
