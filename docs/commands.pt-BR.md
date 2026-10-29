[English](commands.md) | **Português**

# Comandos

Todo comando que imprime um relatório aceita `--json` para saída legível por máquina. As tabelas saem coloridas quando o stdout é um terminal; use `NO_COLOR=1` para desligar as cores (`CLICOLOR_FORCE=1` as força).

| Comando | O que faz |
|---|---|
| `tokred configure` | Integra com o Claude Code |
| `tokred update` | Troca o binário quando o script de instalação o colocou lá; senão, manda reinstalar |
| `tokred unconfigure` | Remove as partes do tokred do Claude Code |
| `tokred uninstall` | Para o proxy e o serviço dele, remove hooks e agentes; o script de desinstalação remove o binário em seguida |
| `tokred doctor` | Mostra o que está instalado e o que precisa de atenção |
| `tokred stats` | Tokens, custo, economia e projeção, no terminal |
| `tokred stats latency` | Tempo que os hooks acrescentaram ao Claude Code |
| `tokred metrics` | Os mesmos números mais um relatório HTML |
| `tokred audit` | Peso do contexto fixo e MCPs sem uso |
| `tokred pricing sync` / `show` | Atualiza ou mostra a tabela de preços |
| `tokred cache stats` / `prune` | Estado do cache em camadas; remove expirados |
| `tokred proxy start` / `stop` / `status` | Executa e inspeciona o [proxy local](proxy.pt-BR.md) opcional |
| `tokred proxy service install` / `uninstall` | Inicia o proxy no login, como serviço de usuário |
| `tokred claude [args...]` | Executa o Claude Code pelo proxy, só nesta execução |
| `tokred routing status` / `on` / `off` / `explain` | [Roteamento de modelo](routing.pt-BR.md) de requisições de subagentes e auxiliares |
| `tokred compression on` / `off` / `status` | Compressão reversível de resultados grandes de ferramentas |
| `tokred suggest` | Sugestões de modelo mais barato para o loop principal, com a evidência |
| `tokred statusline` | Uma linha curta para a status line do Claude Code |
| `tokred hook <evento>` | Interno: chamado pelo Claude Code |
| `tokred mcp` | Interno: o servidor MCP `tokred_retrieve`, iniciado pelo Claude Code (oculto) |

A pasta do Claude Code pode ser dada com `--claude-dir <path>` em todo comando que precisa dela.

## `tokred configure [--claude-dir <path>] [--yes] [--force] [--proxy-port <n>] [--record-content | --no-record-content]`

Integra o tokred com o Claude Code. O script de instalação o executa por você; rode de novo quando quiser.

1. Encontra a pasta do Claude Code, nesta ordem: `--claude-dir`, o valor salvo no `tokred.toml` (`[claude] dir`), `$CLAUDE_CONFIG_DIR`, `~/.claude` (`%USERPROFILE%\.claude` no Windows). Uma pasta é válida quando existe e contém `settings.json` ou `projects/`. Um `--claude-dir` explícito que não seja válido é erro. Se nada for encontrado, um terminal interativo é perguntado pelo path; senão o comando falha e sugere `--claude-dir`.
2. Detecta o rtk e informa que os dois vão coexistir.
3. Mostra o **plano**: hooks a adicionar (`+`), atualizar (`~`) ou manter (`=`), agentes a criar, atualizar ou deixar como estão, e o backup que será feito. Pede confirmação; `--yes` pula a pergunta (obrigatório quando o stdin não é um terminal).
4. Num terminal, faz as duas perguntas sobre o proxy (veja abaixo), a menos que uma flag ou variável já as tenha respondido.
5. Aplica: cria a pasta de dados, o banco e o `tokred.toml`, faz o merge do `settings.json` (backup em `settings.json.tokred.bak`, mantendo só o último) e instala os agentes.

Rodar de novo é seguro: o resultado é o mesmo. Se o `settings.json` não for um JSON válido, o comando aborta e **não escreve nada**. `--force` sobrescreve os arquivos de agente mesmo que você os tenha editado (sem ele, sua versão é mantida e a nova vai para `tokred-x.md.new`).

**A porta do proxy.** `--proxy-port <n>` (ou `TOKRED_PORT`) define `[proxy] bind` como `<host>:<n>` (o host de um valor existente é mantido, padrão `127.0.0.1`). Sem ela, um terminal é perguntado: "Port for the tokred proxy [1029]" (o padrão é a porta salva, senão 1029). Um não-número, uma porta abaixo de 1024 e uma acima de 65535 são rejeitados; o comando tenta abrir `127.0.0.1:<porta>` para ver se está livre e, quando está ocupada, avisa e pergunta de novo. Uma porta em que o proxy desta mesma instalação já escuta é aceita. Uma porta dada por flag não é perguntada; se outra coisa escuta nela, você recebe um aviso (o proxy não inicia até ela ficar livre).

**Gravação de conteúdo.** `--record-content` / `--no-record-content` (ou `TOKRED_RECORD_CONTENT=1|0`) definem `[proxy] log_bodies`. Sem elas, um terminal é perguntado "Record conversation content so the dashboard can show it? [Y/n]", com a explicação de que o texto fica no banco do próprio tokred nesta máquina e de que cabeçalhos e credenciais nunca são gravados (o padrão é o valor salvo, senão sim).

**Sem terminal** (ou com `--yes`) nada é perguntado: uma flag, senão a variável de ambiente, senão o valor que já está no `tokred.toml`, senão o padrão (porta 1029, conteúdo **não** gravado). O que não foi dito nunca é alterado, então a reexecução silenciosa depois de cada atualização mantém as suas escolhas.

## `tokred update [--claude-dir <path>]`

Consulta a última release e age conforme o lugar onde o binário em execução está:

| O binário está | `tokred update` |
|---|---|
| Na pasta de instalação (`$TOKRED_HOME/bin`, onde o script de instalação o coloca) | Baixa o arquivo para esta plataforma da release do GitHub, **verifica o SHA-256**, troca o binário de forma atômica (no Windows o `.exe` em execução é renomeado para `.exe.old` e removido no próximo início) e reaplica a configuração com o binário novo |
| Em qualquer outro lugar (compilado do código-fonte, copiado à mão) | Avisa que existe versão nova e manda reinstalar com o script de instalação |

Depois que o binário muda de versão, o primeiro comando da CLI que você rodar reaplica a configuração em silêncio (hooks nunca fazem isso), mantendo as suas escolhas. Precisa de rede; se recusa a rodar com `TOKRED_OFFLINE` definido. Veja [instalação](installation.pt-BR.md).

## `tokred unconfigure [--claude-dir <path>] [--purge]`

Remove os hooks do tokred do `settings.json` e os arquivos de agente que o tokred instalou e que você não editou. Agentes editados são mantidos e informados. Arquivos sem o prefixo `tokred-` nunca são tocados, e o que o tokred não entende no `settings.json` fica como está. `--purge` também apaga a pasta de dados; ele se recusa a fazer isso se a pasta não contiver `tokred.db` ou `tokred.toml`. O proxy, o serviço dele e o binário não são tocados: isso é o `tokred uninstall`.

## `tokred uninstall [--claude-dir <path>] [--yes] [--purge]`

Tira o tokred da máquina, exceto o próprio binário e a preparação do shell, que o [script de desinstalação](installation.pt-BR.md#desinstalação) remove quando este processo terminar. Em ordem: para o proxy (`proxy stop`), remove o serviço de usuário do proxy (`proxy service uninstall`, só se instalado), `compression off` (só se o servidor MCP está registrado) e `unconfigure` (hooks e agentes; sem pasta do Claude Code esse passo é pulado). Cada passo é feito no melhor esforço: uma falha é mostrada e o passo seguinte continua. Depois mostra o que sobrou para o script (o binário, a entrada no `PATH`, o wrapper `claude`).

Ele pergunta "Remove tokred from this machine? [y/N]" e depois se a pasta de dados deve ser apagada; `--yes` não pergunta nada e **mantém** os dados, `--purge` apaga sem perguntar (ele recusa uma pasta que não pareça do tokred). Sem terminal, exige `--yes`. Responder não sai com código 2 e não muda nada (o script para aí).

## `tokred doctor [--claude-dir <path>] [--json] [--bench]`

Mostra a versão do tokred, o path do binário, as pastas do Claude Code e de dados, a versão do schema do banco, o estado da config, a detecção do rtk, uma tabela de hooks (`ok`, `missing`, `outdated`, ou path de binário que não existe mais), uma tabela de agentes (`current`, `outdated`, `edited by you`, `missing`, `.new` pendente) e uma lista de problemas e observações. Uma seção do proxy mostra se o [proxy](proxy.pt-BR.md) está rodando, seus modos de roteamento e compressão, e se o `ANTHROPIC_BASE_URL` aponta para ele ou para outro gateway. Com `--bench` também executa cada hook muitas vezes com entrada realista e mede a latência real por fora (do início do processo até o fim); o resultado é salvo e usado pelo `stats latency`. `TOKRED_BENCH_RUNS` define as execuções por payload (padrão 50).

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

## `tokred proxy start [--foreground] [--bind <host:porta>]` / `stop` / `status [--json]`

O [proxy local](proxy.pt-BR.md) opcional. `start` o executa em segundo plano (o mesmo binário, `proxy start --foreground`); `--foreground` o mantém no terminal, que é o que o serviço de usuário usa. `--bind` substitui `[proxy] bind` (padrão `127.0.0.1:1029`). `stop` pede que ele encerre e espera até que isso aconteça. `status` mostra se está rodando, se o serviço de usuário está instalado, se o `ANTHROPIC_BASE_URL` deste terminal aponta para ele e os modos de roteamento e compressão.

## `tokred proxy service install` / `uninstall`

Instala o proxy como serviço de usuário que inicia no login (uma unit de usuário do systemd no Linux, um agente do launchd no macOS, uma tarefa de logon no Windows), ou o remove. Nada é instalado a menos que você execute isto.

## `tokred claude [args...]`

Inicia o proxy se ele não estiver rodando e executa o Claude Code por ele, só nesta execução: nada é gravado nas configurações do Claude Code. Todo argumento vai para o `claude` sem alteração e o tokred sai com o código de saída do `claude`. Se o proxy não puder iniciar, ou se o `ANTHROPIC_BASE_URL` já apontar para outro gateway, o Claude Code roda como antes e o tokred explica o motivo no stderr.

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
