[English](commands.md) | **Português**

# Comandos

Referência de todos os comandos do `tokred` e das suas flags. `tokred <comando> --help` mostra as mesmas flags da sua versão.

| Comando | O que faz |
|---|---|
| [`configure`](#configure) | Adiciona os hooks e agentes do tokred ao Claude Code |
| [`unconfigure`](#unconfigure) | Remove os hooks e agentes do tokred do Claude Code |
| [`update`](#update) | Troca o binário por uma versão mais nova (ou uma específica) |
| [`uninstall`](#uninstall) | Remove o tokred da máquina por completo |
| [`doctor`](#doctor) | Mostra o que está instalado e o que precisa de atenção |
| [`stats`](#stats) | Tokens, custo, economia e projeção no terminal |
| [`stats latency`](#stats-latency) | Tempo que os hooks acrescentaram ao Claude Code |
| [`metrics`](#metrics) | Os mesmos números, mais um relatório HTML |
| [`audit`](#audit) | Peso do contexto fixo e dos servidores MCP sem uso |
| [`pricing sync` / `show`](#pricing) | Atualiza ou mostra a tabela de preços |
| [`cache stats` / `prune`](#cache) | Inspeciona o cache em camadas ou remove entradas expiradas |
| [`proxy start` / `stop` / `status`](#proxy-start-stop-restart-status) | Executa e inspeciona o [proxy local](proxy.pt-BR.md) opcional |
| [`proxy enable` / `disable`](#proxy-enable-disable) | Liga ou desliga os recursos do proxy e aplica na hora |
| [`proxy service install` / `uninstall`](#proxy-service) | Inicia o proxy no login, como serviço de usuário |
| [`initialize`](#initialize) | Faz as configurações do Claude Code concordarem com `[proxy] enabled` |
| [`claude`](#claude) | Executa o Claude Code pelo proxy, só nesta execução |
| [`dashboard`](#dashboard) | Abre o [dashboard ao vivo](dashboard.pt-BR.md) do proxy |
| [`routing status` / `on` / `off` / `explain`](#routing) | [Roteamento de modelos](routing.pt-BR.md) das requisições de subagentes e auxiliares |
| [`compression on` / `off` / `status`](#compression) | Compressão reversível de resultados grandes de ferramentas |
| [`suggest`](#suggest) | Sugestões de modelo mais barato para o loop principal |
| [`statusline`](#statusline) | Uma linha curta para a barra de status do Claude Code |
| `hook <evento>`, `mcp` | Internos: iniciados pelo Claude Code, nunca à mão |

## Comportamento comum

- **Pasta do Claude Code.** Os comandos que a leem ou alteram aceitam `--claude-dir <PATH>`. A pasta é a primeira que se aplicar entre: a flag, `[claude] dir` no `tokred.toml`, `$CLAUDE_CONFIG_DIR`, `~/.claude` (`%USERPROFILE%\.claude` no Windows). Ela é válida quando existe e contém `settings.json` ou `projects/`. Um `--claude-dir` explícito que não seja válido é um erro. O [`initialize`](#initialize) e os comandos [`proxy`](#proxy-enable-disable), que editam o `settings.json` que o próprio Claude Code lê, colocam `$CLAUDE_CONFIG_DIR` antes de `[claude] dir`.
- **`--since`.** Um intervalo em horas, dias ou semanas: `12h`, `7d`, `2w`, `30d`.
- **`--json`.** Onde um comando o lista, a saída é legível por máquina. Nem todo comando o tem.
- **Códigos de saída.** `0` sucesso, `1` erro (a mensagem vai para o stderr), `2` para erro de uso e para um `configure` recusado pelo usuário. O `tokred claude` sai com o código do próprio `claude`. O `hook` sempre sai com `0`; o `initialize` só falha com argumentos inválidos.
- **Cores e ambiente.** As cores e as variáveis de ambiente que mudam o comportamento estão em [configuração](configuration.pt-BR.md).

## configure

```sh
tokred configure [--claude-dir <PATH>] [--yes] [--force]
```

Integra o tokred ao Claude Code: os [hooks e agentes](hooks-and-agents.pt-BR.md), mais a pasta de dados, o banco e um `tokred.toml` padrão. Não configura o proxy; isso é feito pelo [`proxy enable`](#proxy-enable-disable).

1. Encontra a pasta do Claude Code (veja acima). Se nenhuma for encontrada, o caminho é perguntado no terminal; sem terminal o comando falha e sugere `--claude-dir`.
2. Imprime o plano: hooks a adicionar (`+`), atualizar (`~`) ou manter (`=`), agentes a criar, atualizar ou deixar como estão, e o backup que será feito.
3. Pede confirmação. Responder não muda nada e sai com o código `2`, que o script de instalação lê como "pare". Sem terminal o comando se recusa a rodar, a menos que você passe `--yes`.
4. Aplica o plano: cria a pasta de dados, o banco e o `tokred.toml`, faz o merge do `settings.json` (backup em `settings.json.tokred.bak`; só o último é mantido) e instala os agentes.

Rodar de novo é seguro. Se o `settings.json` não for um JSON válido, o comando para e não escreve nada. Ele nunca altera um valor de `[proxy]` que você escolheu.

| Flag | Efeito |
|---|---|
| `--claude-dir <PATH>` | Pasta do Claude Code |
| `-y`, `--yes` | Aplica sem perguntar |
| `--force` | Sobrescreve os arquivos de agente mesmo que você os tenha editado (senão a sua versão é mantida e a nova vai para um arquivo `.new`) |

## unconfigure

```sh
tokred unconfigure [--claude-dir <PATH>] [--purge]
```

Remove os hooks do tokred do `settings.json` e os arquivos de agente que o tokred instalou. Mexe só em hooks e agentes: não no proxy, não no `ANTHROPIC_BASE_URL`, não nos seus dados. Agentes editados são mantidos e informados, arquivos sem o prefixo `tokred-` nunca são tocados, e uma atualização não recoloca os hooks: só o `tokred configure` faz isso. Detalhes em [hooks e agentes](hooks-and-agents.pt-BR.md).

| Flag | Efeito |
|---|---|
| `--claude-dir <PATH>` | Pasta do Claude Code |
| `--purge` | Apaga também a pasta de dados do tokred (banco, config, logs, relatórios); recusado quando a pasta não tem nem `tokred.db` nem `tokred.toml` |

## update

```sh
tokred update [--version <VERSION>] [--claude-dir <PATH>]
```

Verifica a última versão estável (uma pré-release nunca é oferecida) e age conforme onde o binário em execução está:

| O binário está | `tokred update` |
|---|---|
| Na pasta de instalação (`$TOKRED_HOME/bin`, onde o script de instalação o coloca) | Baixa o arquivo desta plataforma, verifica o SHA-256, troca o binário e reaplica a configuração com o novo binário |
| Em qualquer outro lugar (compilado do código-fonte, copiado à mão) | Avisa se existe uma versão mais nova e, se existir, manda reinstalar com o script de instalação |

`--version <VERSION>` (um `v` na frente é aceito) instala exatamente essa versão em vez da última estável: use para testar uma release candidate ou para voltar atrás. Ele baixa essa release diretamente e verifica o checksum como qualquer atualização. Uma versão que já está instalada não é baixada de novo. Precisa de rede e se recusa a rodar com `TOKRED_OFFLINE` definido. A configuração é reaplicada com o novo binário e mantém as suas escolhas. Veja [instalação](installation.pt-BR.md).

## uninstall

```sh
tokred uninstall [--claude-dir <PATH>]
```

Remove o tokred da máquina por completo e não pergunta nada. Em ordem:

1. para o proxy e remove o serviço de usuário dele (se instalado);
2. roda `compression off` (se o servidor MCP estiver registrado);
3. remove o `ANTHROPIC_BASE_URL` do tokred do `settings.json` do Claude Code (um gateway seu permanece);
4. remove os hooks e agentes, como o `unconfigure` (ignorado quando não há pasta do Claude Code);
5. apaga a pasta de dados do tokred;
6. executa o script de desinstalação embutido no binário: ele apaga o binário e a pasta dele (quando não há mais nada nela), a entrada no `PATH` e o wrapper `claude` nos perfis do seu shell. Um binário compilado do código-fonte é deixado onde está. No Windows este passo roda logo depois que o comando termina, porque um programa em execução não consegue se apagar.

Cada passo é de melhor esforço: uma falha é impressa e o passo seguinte ainda roda. Se algum passo falhou, o binário é mantido para você corrigir o problema e rodar o comando de novo. Veja [instalação](installation.pt-BR.md).

## doctor

```sh
tokred doctor [--claude-dir <PATH>] [--json] [--bench]
```

Imprime a versão, o caminho do binário, as pastas do Claude Code e de dados, o schema do banco, o estado da config e se há outras ferramentas de hook. Depois, uma tabela de hooks (`ok`, `missing`, `outdated` ou `ok, but binary path is gone`), uma tabela de agentes (`current`, `outdated`, `edited by you`, `missing`, com `.new file pending` quando há um esperando) e a lista de problemas e observações. Uma seção do proxy mostra se o [proxy](proxy.pt-BR.md) está rodando (ou `disabled`), seus modos de roteamento e compressão, o `ANTHROPIC_BASE_URL` no `settings.json` do Claude Code e neste terminal, e avisa quando o `settings.json` aponta para um proxy que não está rodando. O proxy HTTP (só host e porta) e os arquivos de CA extras aparecem quando configurados.

| Flag | Efeito |
|---|---|
| `--claude-dir <PATH>` | Pasta do Claude Code |
| `--json` | Saída legível por máquina |
| `--bench` | Também executa cada hook várias vezes com entrada realista e mede a latência real, do início do processo até a saída. O resultado é salvo e usado pelo [`stats latency`](#stats-latency). `TOKRED_BENCH_RUNS` define as execuções por entrada (padrão 50) |

## stats

```sh
tokred stats [--since <SPAN>] [--by <GROUP>] [--json] [--claude-dir <PATH>]
```

O relatório de custo no terminal: totais, quanto os mesmos tokens custariam no opus, a economia do roteamento e dos guards (uma estimativa, rotulada como tal), a taxa de acerto do cache de prompt, a projeção mensal e uma tabela com coluna de participação e barra de impacto. Como cada número é calculado está em [métricas](metrics.pt-BR.md). Quando o [proxy](proxy.pt-BR.md) registrou algo, uma seção **Proxy** vem em seguida.

| Flag | Efeito |
|---|---|
| `--since <SPAN>` | Até onde olhar para trás (padrão `7d`) |
| `--by <GROUP>` | Agrupa as linhas: `model`, `day`, `project`, `origin` (main, subagent, auxiliary, compaction) ou `agent-type` (tipo de agente de uma requisição de subagente). `origin` e `agent-type` só são exatos com o proxy |
| `--json` | Saída legível por máquina (acrescenta a chave `proxy` quando o proxy registrou algo) |

## stats latency

```sh
tokred stats latency [--since <SPAN>] [--by <GROUP>] [--json] [--claude-dir <PATH>]
```

Quanto tempo os hooks acrescentaram ao Claude Code: execuções, p50/p95/p99/máx por hook, tempo total acrescentado, média por prompt, resultados e um alerta quando o p95 de um hook passa de 50 ms.

| Flag | Efeito |
|---|---|
| `--since <SPAN>` | Até onde olhar para trás (padrão `7d`) |
| `--by <GROUP>` | `hook`, `day` ou `session` |
| `--json` | Saída legível por máquina |

## metrics

```sh
tokred metrics [--since <SPAN>] [--no-open] [--claude-dir <PATH>]
```

Imprime o mesmo relatório do `stats` e grava um relatório HTML autocontido em `<data>/reports/metrics.html` (sempre o mesmo arquivo, sobrescrito), depois o abre no navegador. O relatório funciona offline. `--since` tem padrão `30d`; `--no-open` só grava o arquivo. Veja [métricas](metrics.pt-BR.md).

## audit

```sh
tokred audit [--project <PATH>] [--json] [--claude-dir <PATH>]
```

Estima o peso do contexto que o Claude Code carrega antes de você digitar (arquivos `CLAUDE.md` com seus imports `@`, regras, skills, agentes) e lista os servidores MCP que não foram usados nos últimos 30 dias. `--project <PATH>` escolhe o projeto a auditar (padrão: a pasta atual). As contagens de tokens são uma estimativa: caracteres divididos por 4. O tamanho exato dos esquemas das ferramentas MCP não pode ser calculado localmente, então só o uso é mostrado.

## pricing

```sh
tokred pricing sync
```

```sh
tokred pricing show [--json]
```

`sync` atualiza a tabela de preços a partir da lista pública do LiteLLM; se ela não puder ser alcançada, o snapshot embutido continua em uso. `show` imprime a tabela (USD por milhão de tokens). Os preços também são sincronizados automaticamente, no máximo uma vez por dia, como descrito em [configuração](configuration.pt-BR.md). Como os preços são usados está em [métricas](metrics.pt-BR.md).

## cache

```sh
tokred cache stats [--json]
```

```sh
tokred cache prune
```

`stats` mostra entradas e acertos por camada (L1, L2, L3); `prune` remove as entradas expiradas. Como as camadas funcionam está em [arquitetura](architecture.pt-BR.md).

## proxy start, stop, restart, status

```sh
tokred proxy start [--foreground] [--bind <HOST:PORT>]
```

```sh
tokred proxy stop
```

```sh
tokred proxy restart
```

```sh
tokred proxy status [--json]
```

O [proxy local](proxy.pt-BR.md) opcional. `start` o executa em segundo plano e retorna assim que ele responde (espera até 5 segundos), então `out=$(tokred proxy start)` não trava. `stop` pede que ele encerre e espera até que tenha encerrado. `restart` faz as duas coisas, para que uma configuração que o proxy lê ao iniciar passe a valer (um proxy que não estava rodando apenas é iniciado); ele se recusa enquanto as funcionalidades do proxy estiverem desligadas. `status` mostra se ele está rodando, se o serviço de usuário está instalado, se o `ANTHROPIC_BASE_URL` deste terminal aponta para ele, e os modos de roteamento e compressão. Com `[proxy] enabled = false`, o `start` se recusa; `stop` e `status` continuam funcionando.

| Flag | Efeito |
|---|---|
| `--foreground` | Executa neste terminal em vez de em segundo plano (o que o serviço de usuário usa) |
| `--bind <HOST:PORT>` | Endereço em que escutar (padrão `[proxy] bind`, `127.0.0.1:1029`). Só endereços de loopback são aceitos |
| `--json` | Só no `status`: saída legível por máquina, incluindo o proxy HTTP e os arquivos de CA que o proxy usa |

## proxy enable, disable

```sh
tokred proxy enable [--port <PORT>] [--record-content <BOOL>] [--yes]
```

```sh
tokred proxy disable
```

O único lugar que liga ou desliga os recursos do proxy e aplica na hora. Eles alteram apenas o proxy, a seção `[proxy]` do `tokred.toml` e o `env.ANTHROPIC_BASE_URL` do Claude Code.

`enable` escolhe duas coisas, salva-as com `[proxy] enabled = true` (no lugar: as suas outras chaves permanecem), inicia o proxy e, quando ele responde, define `env.ANTHROPIC_BASE_URL` no `settings.json` do Claude Code com a URL dele.

| Flag | Efeito |
|---|---|
| `--port <PORT>` | Porta em que escutar, de 1024 a 65535 (padrão: a salva, senão `1029`). Também `TOKRED_PORT`; a flag vence. Define `[proxy] bind`, mantendo o host |
| `--record-content <BOOL>` | Deixa o [dashboard](dashboard.pt-BR.md) mostrar o que foi enviado e devolvido: `true`, `false`, `yes`, `no`, `1` ou `0`. Também `TOKRED_RECORD_CONTENT`; a flag vence. Define `[proxy] log_bodies`. O texto fica no banco do próprio tokred, nesta máquina; cabeçalhos e credenciais nunca são gravados |
| `-y`, `--yes` | Não pergunta nada: usa os valores dados, os salvos ou os padrões |

Em um terminal, o que nenhuma flag ou variável informou é perguntado: a porta (uma porta ocupada é informada e perguntada de novo; o proxy desta instalação conta como livre) e se o conteúdo das conversas deve ser gravado (padrão: a resposta salva, senão sim). Sem terminal, ou com `--yes`, nada é perguntado: os valores salvos são mantidos e, quando não há nenhum salvo, a porta é `1029` e o conteúdo **não** é gravado. Um proxy que já está rodando é reiniciado quando a porta ou a resposta sobre gravar o conteúdo mudou, e o comando avisa.

`disable` define `[proxy] enabled = false` e remove a URL do tokred do `settings.json`. Um proxy em execução continua rodando (`tokred proxy stop` o para). Enquanto desligado, `proxy start` e `proxy service install` se recusam, `tokred claude` executa o `claude` diretamente e o `doctor` mostra `disabled`. Um gateway diferente em `ANTHROPIC_BASE_URL` nunca é sobrescrito nem removido: o `enable` então salva a configuração e avisa.

## proxy service

```sh
tokred proxy service install
```

```sh
tokred proxy service uninstall
```

Instala o proxy como serviço de usuário que inicia no login (uma unit de usuário do systemd no Linux, um agente do launchd no macOS, uma tarefa de logon no Windows), ou o remove. Nada é instalado a menos que você rode isto. O `install` se recusa enquanto `[proxy] enabled = false`.

## initialize

```sh
tokred initialize [--quiet | --verbose] [--json]
```

Faz o `settings.json` do Claude Code concordar com `[proxy] enabled`. O wrapper de shell `claude` o executa antes de cada execução do `claude`, então ele é rápido, não imprime nada a menos que haja um problema e sai com `0` mesmo quando não consegue fazer o seu trabalho. Ligado: inicia o proxy quando ele não está rodando e define `env.ANTHROPIC_BASE_URL` quando o proxy responde (ou a remove de novo, com um aviso, se o proxy não conseguir iniciar). Desligado: remove a URL do tokred. Um gateway seu, no arquivo ou exportado no shell, nunca é tocado.

| Flag | Efeito |
|---|---|
| `--quiet` | O padrão: imprime só avisos |
| `-v`, `--verbose` | Diz também o que mudou |
| `--json` | Relatório no stdout: `state`, `started`, `actions`, `warnings` |

## claude

```sh
tokred claude [ARGS...]
```

Inicia o proxy se preciso e executa o Claude Code por ele, só nesta execução: nada é gravado nas configurações do Claude Code. Todo argumento vai para o `claude` sem alteração, exceto `-h`, `--help`, `-V` e `--version`, que o próprio tokred responde (coloque `--` antes deles para repassá-los). Se o proxy não puder iniciar, se o `ANTHROPIC_BASE_URL` já apontar para outro gateway, ou se `[proxy] enabled = false`, o Claude Code roda como antes e o tokred diz o motivo no stderr.

## dashboard

```sh
tokred dashboard [--no-open]
```

Abre o [dashboard ao vivo](dashboard.pt-BR.md) do proxy em execução no navegador. Não inicia nada: se o proxy não estiver rodando, ele avisa. O endereço leva um token de uso único que expira em cerca de um minuto; rode o comando de novo para obter outro. `--no-open` imprime o endereço em vez de abri-lo.

## routing

```sh
tokred routing status [--since <SPAN>] [--json]
```

```sh
tokred routing on
```

```sh
tokred routing off
```

```sh
tokred routing explain <ID>
```

`status` mostra o modo, o que o roteamento fez (ou teria feito, em `observe`) no período (`--since`, padrão `7d`: requisições alteradas, economia estimada e uma projeção mensal, para você decidir se vale ligar) e as decisões recentes. `on` define `[routing] mode = "enforce"` e `off` define `"observe"`: ambos editam o `tokred.toml` e reiniciam um proxy em execução para aplicar. `explain <ID>` diz por que uma decisão foi tomada (os ids estão no `status`). O modelo do loop principal nunca é alterado. Veja [roteamento](routing.pt-BR.md).

## compression

```sh
tokred compression on [--claude-dir <PATH>]
```

```sh
tokred compression off [--claude-dir <PATH>]
```

```sh
tokred compression status
```

`on` registra o servidor MCP `tokred_retrieve` na config de usuário do Claude Code (`.claude.json` na pasta do Claude Code, se existir lá, senão `~/.claude.json`; o arquivo anterior é salvo ao lado como `.tokred.bak`) e define `[compression] mode = "enforce"`: resultados grandes de ferramentas são trocados por uma prévia e o modelo lê o original de volta com `tokred_retrieve`. `off` remove o registro do servidor e define `"observe"` (só mede). Reinicie o proxy e o Claude Code para aplicar o `on`. `status` mostra o modo, se o servidor está registrado e se o disjuntor disparou. Nada é editado a menos que você rode `on` ou `off`.

## suggest

```sh
tokred suggest [--since <SPAN>] [--json]
```

Quantos prompts o proxy acompanhou, quantos eram simples, e as últimas sugestões de um modelo mais barato para o loop principal, com as evidências. Ele só aconselha; trocar é sempre decisão sua (`/model`). Precisa do proxy. `--since` tem padrão `7d`.

## statusline

```sh
tokred statusline
```

Lê o JSON da barra de status do Claude Code no stdin e imprime uma linha curta: a sugestão pendente e alguns números da sessão. Defina-o como o comando da sua barra de status se quiser.

## Comandos internos

`tokred hook <evento>` é chamado pelo Claude Code, nunca à mão (veja [hooks e agentes](hooks-and-agents.pt-BR.md); para testar, veja [desenvolvimento](development.pt-BR.md)). Ele lê JSON no stdin, escreve JSON no stdout e sempre sai com `0`, até para um evento desconhecido. `tokred mcp` é o servidor MCP `tokred_retrieve` que o Claude Code inicia quando a compressão está ligada.
