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
| `tokred proxy start` / `stop` / `status` | Executa e inspeciona o [proxy local](proxy.pt-BR.md) opcional |
| `tokred proxy enable` / `disable` | Liga ou desliga os recursos do proxy (`[proxy] enabled`) e aplica na hora |
| `tokred proxy service install` / `uninstall` | Inicia o proxy no login, como serviço de usuário |
| `tokred initialize` | Faz as configurações do Claude Code concordarem com `[proxy] enabled`; executado pelo wrapper de shell do `claude` antes de toda abertura |
| `tokred claude [args...]` | Executa o Claude Code pelo proxy, só nesta execução |
| `tokred routing status` / `on` / `off` / `explain` | [Roteamento de modelo](routing.pt-BR.md) de requisições de subagentes e auxiliares |
| `tokred compression on` / `off` / `status` | Compressão reversível de resultados grandes de ferramentas |
| `tokred suggest` | Sugestões de modelo mais barato para o loop principal, com a evidência |
| `tokred statusline` | Uma linha curta para a status line do Claude Code |
| `tokred hook <evento>` | Interno: chamado pelo Claude Code |
| `tokred mcp` | Interno: o servidor MCP `tokred_retrieve`, iniciado pelo Claude Code (oculto) |

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

Remove os hooks do tokred do `settings.json` (e o `ANTHROPIC_BASE_URL` do tokred, quando foi gravado ali; o seu próprio gateway fica) e os arquivos de agente que o tokred instalou e que você não editou. Agentes editados são mantidos e informados. Arquivos sem o prefixo `tokred-` nunca são tocados, e o que o tokred não entende no `settings.json` fica como está. `--purge` também apaga a pasta de dados; ele se recusa a fazer isso se a pasta não contiver `tokred.db` ou `tokred.toml`.

## `tokred doctor [--claude-dir <path>] [--json] [--bench]`

Mostra a versão do tokred, o path do binário, as pastas do Claude Code e de dados, a versão do schema do banco, o estado da config, a detecção do rtk, uma tabela de hooks (`ok`, `missing`, `outdated`, ou path de binário que não existe mais), uma tabela de agentes (`current`, `outdated`, `edited by you`, `missing`, `.new` pendente) e uma lista de problemas e observações. Uma seção do proxy mostra se o [proxy](proxy.pt-BR.md) está rodando (ou "disabled" quando `[proxy] enabled = false`), seus modos de roteamento e compressão, o `ANTHROPIC_BASE_URL` do `settings.json` do Claude Code e deste terminal, e avisa quando o settings.json aponta para um proxy que não está rodando. Quando `HTTPS_PROXY` está definida ou há arquivos de CA extras configurados, mostra também o proxy (só host e porta, nunca as credenciais) e os arquivos de CA. Com `--bench` também executa cada hook muitas vezes com entrada realista e mede a latência real por fora (do início do processo até o fim); o resultado é salvo e usado pelo `stats latency`. `TOKRED_BENCH_RUNS` define as execuções por payload (padrão 50).

## `tokred stats [--since 7d] [--by model|day|project|origin|agent-type] [--json]`

O painel de custo, impresso no terminal: totais (mensagens, tokens, custo real), quanto os mesmos tokens custariam se tudo rodasse no opus, a economia do roteamento e das guardas (**uma estimativa**, rotulada como tal), a taxa de acerto do cache de prompt, a projeção mensal, um medidor de eficiência e uma tabela numerada com coluna de participação e barra de impacto. Modelos precificados por família aparecem sinalizados. Quando há dados anteriores ao primeiro `configure`, compara o custo por sessão antes e depois. Os transcripts são ingeridos de forma incremental antes, então fica rápido depois da primeira vez. `--since` aceita horas, dias e semanas (`12h`, `7d`, `2w`).

Com o [proxy](proxy.pt-BR.md) registrando, os números vêm dos seus dados exatos por requisição (mesclados com os transcripts para o que ele não viu, sem contar nada duas vezes; veja [metrics](metrics.pt-BR.md)), `--by origin` e `--by agent-type` agrupam o custo por quem enviou a requisição, e segue uma seção **Proxy**: fonte dos dados, overhead contra o orçamento de 10 ms, decisões de roteamento com a economia estimada, o modelo pedido contra o que respondeu, cache misses do prompt por causa e a última sugestão ainda não mostrada. O `--json` acrescenta a chave `proxy`.

## `tokred stats latency [--since 7d] [--by hook|day|session] [--json]`

Quanto tempo os hooks do tokred acrescentaram ao Claude Code: execuções, p50/p95/p99/máximo por hook, tempo total acrescentado (uma estimativa: inclui o custo de iniciar o processo medido pelo `doctor --bench`), média por prompt, resultados (ok, erro, panic, timeout guard), um alerta quando o p95 de um hook passa do orçamento de 50 ms e a comparação com o que as guardas economizaram no mesmo período.

## `tokred metrics [--since 30d] [--no-open]`

Imprime o mesmo painel do `stats` (com a seção Proxy quando o proxy registrou algo) e grava o relatório HTML autocontido em `<dados>/reports/metrics.html` (sempre o mesmo arquivo, sobrescrito de forma atômica), e o abre no navegador a menos que se passe `--no-open`. O relatório funciona offline: sem rede, sem fontes nem scripts externos.

## `tokred audit [--project <path>] [--json]`

Estima (caracteres / 4) o peso do contexto que o Claude Code carrega antes de você digitar: arquivos `CLAUDE.md` com seus imports `@`, rules, skills e agentes; e lista os servidores MCP sem uso nos últimos 30 dias. O projeto é a pasta atual por padrão. O peso exato dos schemas das ferramentas MCP não pode ser calculado localmente, então só o uso é mostrado.

## `tokred pricing sync` / `tokred pricing show [--json]`

`sync` atualiza a tabela de preços a partir da lista pública do LiteLLM; se ela não puder ser alcançada, o snapshot embutido continua em uso. `show` imprime a tabela (USD por milhão de tokens). Os preços também são sincronizados automaticamente, no máximo uma vez por dia, pelos comandos acima.

## `tokred cache stats [--json]` / `tokred cache prune`

`stats` mostra entradas e acertos por camada (L1/L2/L3); `prune` remove os expirados. Veja [cache](cache.pt-BR.md).

## `tokred proxy start [--foreground] [--bind <host:porta>]` / `stop` / `status [--json]`

O [proxy local](proxy.pt-BR.md) opcional. `start` o executa em segundo plano (o mesmo binário, `proxy start --foreground`); `--foreground` o mantém no terminal, que é o que o serviço de usuário usa. `--bind` substitui `[proxy] bind` (padrão `127.0.0.1:1029`). `stop` pede que ele encerre e espera até que isso aconteça. `status` mostra se está rodando, se o serviço de usuário está instalado, se o `ANTHROPIC_BASE_URL` deste terminal aponta para ele e os modos de roteamento e compressão. Com `--json`, o campo `network` traz o proxy HTTP (só host e porta) e os arquivos de CA extras que o proxy usa. Ele é um processo em segundo plano que não mantém os pipes de saída de quem o chamou, então `out=$(tokred proxy start)` retorna (no Windows ele é iniciado pelo `Start-Process` do PowerShell por esse motivo).

## `tokred proxy enable` / `disable`

Definem `[proxy] enabled` no `tokred.toml` e aplicam na hora. O `enable` sobe o proxy se preciso e, quando ele responder, grava no `settings.json` do Claude Code o `env.ANTHROPIC_BASE_URL` com a URL do proxy. O `disable` remove dali a URL do tokred e deixa em execução um proxy que esteja rodando (`tokred proxy stop` o para). Enquanto desabilitado, `proxy start` e `proxy service install` se recusam, o `tokred claude` executa o `claude` direto e o `doctor` mostra "disabled"; `proxy stop` e `proxy status` continuam funcionando. Outro gateway em `ANTHROPIC_BASE_URL` nunca é sobrescrito nem removido: nesse caso o `enable` só salva a configuração e avisa. Veja [proxy](proxy.pt-BR.md).

## `tokred initialize [--quiet | --verbose] [--json]`

Reconcilia o `settings.json` do Claude Code com `[proxy] enabled`; o wrapper de shell do `claude` o executa antes de toda abertura, por isso ele foi feito para ser rápido e nunca atrapalhar. Lê um arquivo e sonda o proxy uma vez (sem banco de dados), não imprime nada a menos que haja um problema, e sempre sai com 0 (só argumentos inválidos falham). Habilitado: sobe o proxy quando ele não está rodando e grava o `env.ANTHROPIC_BASE_URL` quando ele responder, ou o remove de novo com um aviso se o proxy não puder subir. Desabilitado: remove a URL do tokred. Outro gateway, no arquivo ou exportado no shell, nunca é tocado e nada é iniciado. `--quiet` é o padrão; `--verbose` diz o que mudou; `--json` imprime um relatório (`state`, `started`, `actions`, `warnings`). As regras estão em [proxy](proxy.pt-BR.md).

## `tokred proxy service install` / `uninstall`

Instala o proxy como serviço de usuário que inicia no login (uma unit de usuário do systemd no Linux, um agente do launchd no macOS, uma tarefa de logon no Windows), ou o remove. Nada é instalado a menos que você execute isto. O `install` se recusa enquanto `[proxy] enabled = false`.

## `tokred claude [args...]`

Inicia o proxy se ele não estiver rodando e executa o Claude Code por ele, só nesta execução: nada é gravado nas configurações do Claude Code. Todo argumento vai para o `claude` sem alteração e o tokred sai com o código de saída do `claude`. Se o proxy não puder iniciar, ou se o `ANTHROPIC_BASE_URL` já apontar para outro gateway, ou se `[proxy] enabled = false`, o Claude Code roda como antes e o tokred explica o motivo no stderr.

## `tokred routing status [--json]` / `on` / `off` / `explain <id>`

`status` mostra o modo, o circuit breaker e as decisões recentes. `on` define `[routing] mode = "enforce"` e `off` define `observe` (ambos editam o `tokred.toml`; reinicie o proxy para aplicar). `explain <id>` diz por que uma decisão foi tomada. O modelo do loop principal nunca é alterado. Veja [roteamento](routing.pt-BR.md).

## `tokred compression on` / `off` / `status`

`on` registra o servidor MCP `tokred_retrieve` na configuração de usuário do Claude Code (`~/.claude.json`, com o conteúdo anterior salvo em `.tokred.bak`) e define `[compression] mode = "enforce"`: resultados grandes de ferramentas são trocados por uma prévia e o modelo lê o original de volta com `tokred_retrieve`. `off` remove o registro do servidor e define `observe` (só mede). `status` mostra o modo, se o servidor está registrado e se o circuit breaker disparou. Nada é editado a menos que você execute `on` ou `off`.

## `tokred suggest [--since 7d] [--json]`

As sugestões para o loop principal: quantos prompts o proxy acompanhou, quantos eram simples e as últimas sugestões de um modelo mais barato. Só aconselha; trocar é sempre decisão sua (`/model`). Precisa do proxy.

## `tokred statusline`

Lê o JSON da status line do Claude Code no stdin e imprime uma linha curta (a sugestão pendente e alguns números da sessão). Configure-o como comando da status line se quiser.

## `tokred hook <evento>` (interno)

Chamado pelo Claude Code, nunca à mão (exceto para testes, veja [desenvolvimento](development.pt-BR.md)). Lê JSON do stdin e escreve JSON no stdout. Eventos: `route`, `enforce`, `guard-read`, `guard-search`, `session-check`, `session-start`. **Sempre sai com código 0**, mesmo para evento desconhecido ou entrada inválida. Veja [hooks](hooks.pt-BR.md).

O `tokred mcp` também é interno (oculto no `--help`): o servidor MCP com a ferramenta `tokred_retrieve` que o Claude Code inicia quando a compressão está ligada.
