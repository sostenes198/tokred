[English](decisions.md) | **Português**

# Registro de decisões

As decisões não óbvias por trás do tokred, com o motivo de cada uma. Formato: data, contexto, decisão, consequências. Onde a documentação oficial do Claude Code difere de uma suposição anterior, a documentação vence e a divergência fica registrada aqui. Uma entrada que uma posterior substitui é reduzida a uma linha e um ponteiro.

## 2026-10-28 — A ferramenta de subagentes é `Agent`, matcher `Agent|Task`

- **Contexto:** a documentação de sub-agents chama a ferramenta de `Agent` (renomeada de `Task` no Claude Code v2.1.63), com `Task(...)` ainda aceito como alias.
- **Decisão:** o hook `enforce` usa o matcher `Agent|Task` (a sintaxe de matcher aceita nomes exatos separados por `|`).
- **Consequências:** funciona em versões antigas e novas do Claude Code.

## 2026-10-28 — O `timeout` dos hooks é em segundos

- **Contexto:** a documentação de hooks define `timeout` em segundos (padrão 600 para hooks de comando).
- **Decisão:** todo hook do tokred é registrado com `timeout: 5`.

## 2026-10-28 — Comandos de hook do tokred são reconhecidos estruturalmente, não pela substring `tokred hook `

- **Contexto:** as entradas que o tokred grava têm a forma `"/opt/homebrew/bin/tokred" hook guard-read`: a aspa de fechamento fica entre `tokred` e `hook`, então a substring `tokred hook ` nunca aparece nelas. Casar por ela nunca encontraria (e portanto nunca substituiria nem removeria) as entradas do próprio tokred.
- **Decisão:** um comando é do tokred quando executa um executável cujo nome de arquivo começa com `tokred` (`tokred`, `tokred.exe`, ou uma cópia versionada ou renomeada como `tokred-0.4.0`), opcionalmente entre aspas, seguido de ` hook <nome>`. Um teste unitário descobriu isso: sob `cargo test` o binário se chama `tokred-<hash>`, e uma regra de nome exato fazia o `configure` duplicar entradas e o `unconfigure` não remover nada.
- **Consequências:** idempotência e unconfigure funcionam com caminhos entre aspas, caminhos do Windows com espaços, binários renomeados e um `tokred hook x` simples. Um wrapper como `bash -c 'tokred hook x'` deliberadamente não é reconhecido.

## 2026-10-28 — Hooks nunca falham e os rótulos do formato de retorno estão em inglês

- **Decisão:** `tokred hook <evento>` é despachado **antes** da interpretação dos argumentos e sempre sai com 0, até para um evento desconhecido, porque o Claude Code trata o código de saída 2 como "bloquear". O formato de retorno dos agentes usa os rótulos em inglês `RESULT`, `FILES` e `PENDING`.

## 2026-10-28 — A versão mínima do Rust é 1.89

- **Contexto:** a edição 2024 exige 1.85, mas o `rusqlite` 0.41 declara 1.89.
- **Decisão:** `rust-version = "1.89"`, verificado com `cargo +1.89 check --locked` localmente e no CI.

## 2026-10-28 — Pasta de dados vem de `BaseDirs`, não de `ProjectDirs`

- **Contexto:** no Windows o `ProjectDirs` acrescenta `\data` (`%APPDATA%\tokred\data`), que não é a pasta que queremos.
- **Decisão:** usar `BaseDirs::data_dir().join("tokred")`, que dá `~/.local/share/tokred`, `~/Library/Application Support/tokred` e `%APPDATA%\tokred`.

## 2026-10-28 — `option-ext` (MPL-2.0) permitido por uma exceção por crate

- **Contexto:** o `directories` puxa o `dirs-sys`, que puxa o `option-ext`, licenciado como MPL-2.0, que não está na lista de licenças permitidas (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib).
- **Decisão:** manter o `directories` e acrescentar uma exceção do `cargo-deny` para esse único crate. A MPL-2.0 é copyleft em nível de arquivo e o crate é usado sem modificação. O `deny.toml` também permite `CDLA-Permissive-2.0`, os dados de certificados de CA da Mozilla embutidos pelo `webpki-roots`.

## 2026-10-28 — Detalhes do `configure` / `unconfigure`

- `--claude-dir` é aceito por `configure`, `unconfigure` e `doctor`; um caminho explícito mas inválido é um erro e não recorre a outros candidatos. O `configure` também aceita `--force`.
- Um `settings.json` em branco conta como `{}`; JSON inválido aborta sem gravar nada, sem backup e sem criar a pasta de dados.
- O `settings.json` só é reescrito quando o resultado do merge difere, e é reserializado com indentação de 2 espaços (a ordem das chaves é preservada, o espaçamento original não). O backup guarda o texto anterior exato.
- Limite do round-trip: um array de evento ou objeto `hooks` que **já estava vazio** e no qual o tokred fez merge é removido pelo `unconfigure`, porque "esvaziado pela nossa remoção" não se distingue de "vazio desde o início". Contêineres vazios que o tokred não tocou são preservados. Se o `settings.json` não existia, o `unconfigure` deixa `{}`.
- O `unconfigure` mantém arquivos de agente que você editou (e, quando o banco sumiu, todos os arquivos de agente, já que a origem não pode ser verificada). Ele é tolerante: remove os handlers do tokred e deixa exatamente como está tudo o que não entende. Só o `configure`, que precisa escrever nesses lugares, reporta um formato inválido.
- `--purge` se recusa a apagar uma pasta que não tem nem `tokred.db` nem `tokred.toml`.
- O banco só é criado depois que o plano é confirmado. O `tokred.toml` só é reescrito (sem comentários) quando faltavam chaves; do contrário, fica byte a byte igual.
- O I/O de terminal passa por uma pequena costura `Io`, e a saída dos comandos é montada por funções puras em `String`, então prompts, texto do plano e cada estado do `doctor` são testados por unidade sem terminal.

## 2026-10-28 — Cache de respostas e resumos de artefatos: medidos, não construídos

- **Decisão:** um cache de respostas de correspondência exata e resumos guardados de artefatos não são construídos; o proxy só mede quantas requisições são repetições exatas (`[shadow] response_cache`). Prompts de agentes raramente se repetem, hooks não conseguem interceptar chamadas de API e a Anthropic já faz cache do prefixo do prompt no servidor. Veja as entradas do proxy abaixo para o que foi construído no lugar.

## 2026-10-28 — Hooks: onde a documentação oficial decide

Confirmado em <https://code.claude.com/docs/en/hooks>:

- O modo `warn` emite só `additionalContext`. Nunca emite `permissionDecision: "allow"`, porque isso pularia o prompt de permissão do próprio usuário.
- O `Stop` emite só `systemMessage` (uma mensagem para o usuário): `additionalContext` ou `decision` no `Stop` fariam o Claude continuar. O `SessionStart` não imprime nada, porque seu stdout vira contexto do modelo.
- O `source` do `SessionStart` tem um quinto valor, `fork`; como `startup` e `resume`, não é um corte (só `clear` e `compact` são).
- Leituras de arquivo são rastreadas por `sessão/agent_id`, porque o contexto de um subagente é separado do pai. Uma leitura no mesmo segundo de um corte é permitida (`>` estrito).
- Globs de negação casam o caminho relativo ao `cwd` da sessão. Imagens e PDFs ficam isentos da checagem de tamanho. Um `Read` com qualquer argumento além de `file_path`, `offset` e `limit` (como `pages`) pula a checagem de releitura. Os bytes evitados por uma leitura parcial são estimados em 80 bytes por linha, e arquivos acima de 16 MiB não são calculados por hash.
- O `guard-search` conta arquivos por raiz de repositório (em cache no cache em camadas, com chave de raiz, limite e mtime da raiz), para de contar em limite + 1 e desiste após 100 ms, caso em que permite a busca.
- O `route` guarda só classes acionáveis em `classifications` (nunca `other`), faz cache das classificações por `blake3(prompt)` mais uma versão de regras e se comporta como `warn` no modo `enforce`. Para o `enforce`, arquiteto e planejador são intercambiáveis.
- O `session-check` avisa uma vez por sessão, de novo depois que o contexto cresce metade do limite, ou depois de um clear ou compact. Um `~/` literal em `transcript_path` não é expandido.
- O stdout não pode ser fechado (`unsafe` é proibido): a resposta é descarregada e a linha do banco é gravada por último, mas o Claude Code só vê o fim quando o processo sai. Um hook nunca cria a pasta de dados nem o `tokred.db`; um `tokred.toml` inválido significa os padrões. Um watchdog (`TOKRED_HOOK_GUARD_MS`, padrão 2 s) transforma um hook travado em um resultado `timeout_guard`.
- Medido em um Mac de desenvolvimento sossegado com build de release: p95 de spawn até saída de 10 a 12 ms (cerca de 7 ms disso são a base do spawn), p95 interno de até cerca de 6 ms; sob carga o p95 chegou a 37 ms. Tudo dentro do orçamento de 50 ms.

## 2026-10-28 — Preços, atualização e o caminho da autoatualização

- O snapshot de preços mantém o formato do próprio LiteLLM com chaves normalizadas, então um único parser lê tanto a lista ao vivo quanto o arquivo embutido (regenerado por `scripts/update-price-snapshot.sh`). Preferência de chaves: chaves Anthropic simples, depois `anthropic/`, `vertex_ai/`, `bedrock/`, depois o `anthropic.` nativo do Bedrock; chaves regionais e de revendedores são descartadas. Preços de cache ausentes são derivados (escrita 1,25x, leitura 0,1x da entrada).
- "Último preço de uma família" é a entrada com a maior versão, depois a data mais recente, no id do modelo (veja a entrada de cache de 1 hora abaixo).
- Uma sincronização que não devolve linhas nunca substitui a tabela. `price_sync_attempt_at` é gravado para que uma sincronização com falha não seja repetida a cada comando.
- `BinarySwap` é uma porta de propriedade do `tokred-update` (a porta `FileSystem` não tem rename nem chmod). A autoatualização verifica o sidecar SHA-256 antes de tocar em qualquer coisa e substitui o binário de forma atômica; no Windows, renomeia o `.exe` antigo para `.old` e o remove na execução seguinte. O Linux sempre mapeia para o build musl. As tags de release são `v<versão>`.
- Depois de uma autoatualização, a configuração é reaplicada executando o binário **novo** (`configure --yes`), então os templates de agentes novos são os instalados.
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL` e `TOKRED_RELEASES_URL`: uma chave para todas as chamadas de rede, e substituições de URL para espelhos e para testes contra um servidor local.

## 2026-10-28 — Cálculos das métricas

- O contrafactual "tudo em opus" usa o preço do opus **mais recente** da tabela. Quando o modelo realmente usado é um opus mais antigo e mais caro, o contrafactual pode ficar abaixo do custo real; a economia de roteamento é então negativa, o que é correto.
- Um "bloqueio" de guarda é um evento `guard-*` com decisão `deny`; `warn` não economiza nada. As economias das guardas são sempre rotuladas como estimativa. O modelo da sessão para precificá-las é o que tem mais mensagens (empates: menor id; reserva opus).
- O divisor da projeção é `min(janela, dias desde a primeira linha)`, no mínimo 1. `TimeRange::last` é `[agora - intervalo, agora + 1)`.
- Um arquivo substituído por um maior não é detectado pela ingestão porque não há hash de conteúdo; um arquivo que encolheu recomeça do zero.
- As classificações do `route` são contadas a partir dos eventos de hook `route`, então não há tabela de classificação separada para consultar no relatório.

## 2026-10-28 — Auditoria: locais confirmados na documentação oficial

- Servidores MCP não são configurados no `settings.json`. O escopo de usuário é o `mcpServers` de nível superior do `~/.claude.json`, o escopo local é `projects["<caminho>"].mcpServers` no mesmo arquivo, o escopo de projeto é `<projeto>/.mcp.json`; a precedência é local, depois projeto, depois usuário. O `settings.json` só contribui com `disabledMcpjsonServers`. Se o `.claude.json` acompanha o `CLAUDE_CONFIG_DIR` não está documentado, então tenta-se primeiro `<claude>/.claude.json`, depois `~/.claude.json`.
- A memória de projeto também inclui `.claude/CLAUDE.md`, `CLAUDE.local.md`, diretórios ancestrais e arquivos `.claude/rules` sempre ativos. Os imports seguem o limite documentado de 4 saltos e ignoram blocos de código, spans de código e comentários HTML em bloco.
- O conselho é "mover para regras por caminho ou uma skill", não "dividir com `@import`", porque imports não reduzem o custo de contexto. Corpos de skills só carregam ao serem invocadas, então as skills também informam o peso de inicialização. Limites: 200 linhas para o `CLAUDE.md` e 500 linhas para skills (ambos da documentação), e 2000 tokens para agentes (uma heurística do tokred).
- Skills, agentes e servidores de plugins, `commands/*.md`, conectores do claude.ai e servidores gerenciados não são enumerados; o uso visto nas transcrições para servidores não configurados localmente é listado à parte.

## 2026-10-28 — Arquivos do release e templates

- Os templates de agentes ficam em `crates/tokred-install/templates/` porque um `include_str!` a partir da raiz do repositório quebraria um crate publicado.
- O pipeline de release e os alvos mudaram depois: veja "Instalação: scripts e arquivos do release, sem gerenciadores de pacotes" e "Sem build nativo para Windows ARM".

## 2026-10-28 — Dashboards de terminal, tarefas em segundo plano e o benchmark

- `tokred stats`, `stats latency` e `metrics` imprimem um dashboard colorido no terminal (o relatório HTML são os mesmos dados, não um substituto). A cor segue `NO_COLOR` (sempre vence), `CLICOLOR_FORCE`, um `TERM` burro e se o stdout é um terminal.
- Avisos de tarefas em segundo plano vão para o **stderr**, para que a saída `--json` fique limpa. As tarefas rodam só depois de comandos que não são hooks, são de melhor esforço e nunca fazem o comando falhar: a sincronização diária de preços, a verificação diária de versão e a reaplicação silenciosa da configuração depois de uma atualização.
- `doctor --bench` executa cada hook como um processo separado em uma pasta de dados temporária com banco próprio (assim os hooks fazem trabalho real e os dados do usuário ficam intactos), mede do spawn até a saída e guarda a diferença em relação às medições dos próprios hooks como a sobrecarga de inicialização do processo em `meta`. `TOKRED_BENCH_RUNS` substitui o padrão de 50 execuções por carga.
- A cobertura de linhas é lida das visões por linha (`lcov`, texto anotado), não do resumo do LLVM, que conta regiões de macro e ramos de erro do `?` separadamente e pode ler uma fração de ponto percentual a menos.

## 2026-10-28 — O proxy, e uma política de modelos que respeita a escolha do usuário

- **Contexto:** o guia oficial de gateways, a página de prompt caching e a página de thinking preservado mostraram que (a) o Claude Code rotula requisições (`x-claude-code-request-class`, tipo de agente, id do agente, id do prompt, id da sessão), (b) gateways são orientados a inspecionar sem modificar e um histórico reescrito pode causar um erro "bound to a different conversation", (c) trocar o modelo principal no meio da sessão reconstrói todo o cache de prompt e (d) requisições auxiliares podem rodar no modelo principal e algumas reaproveitam o cache da conversa.
- **Decisão (combinada com o autor):** o proxy classifica requisições e pode escolher o modelo **apenas** para subagentes e requisições auxiliares. O **loop principal mantém o modelo configurado pelo usuário**; o tokred só **resume e sugere** um mais barato quando a tarefa era simples, de preferência quando o cache já está frio. Compressão e cache de respostas só medem até que os dados justifiquem mais. Todo comportamento que altera o tráfego começa em modo `observe`.

## 2026-10-28 — Sonda do proxy, primeiros resultados (login por chave de API)

Executada com um proxy descartável de repasse (não mantido no repositório; o que ele descobriu é esta entrada) e o Claude Code 2.1.312 de verdade em modo print, com um login por chave de API.

- **O repasse funciona.** O streaming (SSE) é encaminhado sem alteração e o `usage` é legível em `message_start` e `message_delta`. O `model` da resposta bate com o da requisição.
- **Cabeçalhos realmente enviados:** `x-claude-code-session-id` em toda requisição e `x-claude-code-agent-id` só em requisições de subagente. `x-claude-code-request-class`, `x-claude-code-agent-type` e `x-claude-code-prompt-id`, que o guia de gateways diz que são enviados, **não** apareceram nesta versão e neste modo, então a classificação não pode depender deles. Principal versus subagente se distingue pela presença do `agent-id`; o tipo de subagente, pelo conjunto de ferramentas (o subagente Explore tinha 10 ferramentas só de leitura contra 15 do loop principal) e pelos blocos de sistema.
- **Subagentes montam o próprio cache** (a primeira volta gravou cerca de 19,9 mil tokens, as voltas seguintes os leram de volta), e por isso o modelo de um subagente é decidido uma vez e mantido.
- **Reescrever o modelo de requisições de subagente funciona.** Com o loop principal no Sonnet, reescrever só as requisições do subagente para o Haiku foi aceito pela API nas três voltas (incluindo resultados de ferramentas devolvidos), o Claude Code não teve problema com uma resposta de outro modelo e a resposta final estava correta. As requisições do loop principal nunca foram tocadas.
- **Decisão:** o modelo do loop principal **nunca** é trocado, sem nenhuma opção de configuração para isso; o tokred só aconselha.

## 2026-10-28 — Sonda do proxy, resultado com assinatura (login claude.ai)

Mesma sonda e mesma versão do Claude Code, com um login de **assinatura** claude.ai (`ANTHROPIC_API_KEY` não definida), só por `ANTHROPIC_BASE_URL`.

- **O repasse funciona com assinatura.** Status 200 com streaming. A credencial viaja no cabeçalho `authorization` (OAuth), não em `x-api-key`, e o valor `oauth-2025-04-20` do `anthropic-beta` está presente, então um proxy precisa encaminhar `authorization` e o cabeçalho `anthropic-beta` inteiro sem alteração, como diz o guia de gateways. A requisição também levava `extended-cache-ttl-2025-04-11` (o cache de 1 hora).
- **Os cabeçalhos são os mesmos de uma chave de API.**
- **Reescrever o modelo de um subagente funciona com assinatura** (Sonnet para Haiku, status 200 todas as vezes, resposta correta).
- **Não coberto por teste:** os termos de uso da Anthropic para tráfego de assinatura por um proxy (veja a entrada abaixo), os limites do plano por modelo e sessões muito longas.
- **Consequência:** tecnicamente liberado para os dois tipos de login. O proxy continua opcional.

## 2026-10-28 — Sonda do proxy, modo interativo: como reconhecer requisições auxiliares

`claude` interativo (2.1.312, login por chave de API) pela sonda, dois prompts curtos.

- **O `x-claude-code-request-class` também não é enviado no modo interativo.** Só chegou o `x-claude-code-session-id` (e o `x-claude-code-agent-id` para subagentes).
- **A requisição auxiliar é reconhecível pelo formato.** Além das voltas do loop principal (16 ferramentas, thinking adaptativo), uma requisição tinha **nenhuma ferramenta, nenhum thinking e exatamente uma mensagem** (muito provavelmente o título da sessão). Ela rodou no modelo principal e **não** leu nem gravou cache de prompt, então movê-la para um modelo mais barato não perde nada.
- **Verificações de inicialização:** doze requisições com `max_tokens: 1`, sem ferramentas e sem prompt de sistema devolveram `429` e foram repetidas pelo Claude Code; o proxy precisa (e consegue) repassar o `429` sem alteração.
- **Decisão:** uma requisição é auxiliar só quando não tem `agent-id`, nem ferramentas, nem `thinking` e tem exatamente uma mensagem (ou quando um Claude Code futuro enviar `x-claude-code-request-class: auxiliary`). O loop principal sempre leva ferramentas, então nunca pode casar. Todo o resto sem `agent-id` é o loop principal e nunca é reescrito.

## 2026-10-28 — Gravações de cache de 1 hora e ranking de famílias de modelos

- **Gravações de cache de 1 hora.** As transcrições atuais dividem `cache_creation_input_tokens` em `cache_creation.ephemeral_5m_input_tokens` e `ephemeral_1h_input_tokens`, e na máquina da sonda toda gravação era de 1 hora. O LiteLLM publica o preço como `cache_creation_input_token_cost_above_1hr`. O `Usage` ganhou `cache_write_1h` (parte de `cache_write`) e o `ModelPrice` ganhou `cache_write_1h`; quando a lista de preços não o tem, usa-se a taxa documentada de 2x a entrada. A migração v3 acrescenta as colunas e zera os offsets de ingestão para que as linhas existentes sejam preenchidas sem duplicar.
- **Ranking de famílias.** Versões vêm antes de datas (`claude-opus-5-5` > `claude-opus-4-7-20260416`). A regra anterior, que colocava ids sem data abaixo dos com data, fazia o contrafactual precificar tudo em um modelo mais antigo.
- **Fable e Mythos** são famílias. O tipo `Family` continua sendo um enum fechado (famílias desconhecidas ainda resolvem para "sem preço"), o que mantém o casamento exaustivo.

## 2026-10-28 — Tráfego de assinatura pelo proxy: o que dizem os termos

Conferido com os Termos do Consumidor da Anthropic (Seção 3, item 7) e a documentação de gateways do Claude Code. É uma leitura do texto publicado, não aconselhamento jurídico.

- Os Termos do Consumidor proíbem acessar os Serviços "por meios automatizados ou não humanos", exceto com uma chave de API "ou onde permitirmos explicitamente de outra forma". Eles não mencionam proxies ou gateways locais pelo nome.
- A documentação do Claude Code descreve explicitamente a configuração que o tokred usa: com apenas `ANTHROPIC_BASE_URL` definida, "um login claude.ai salvo continua sendo a credencial ativa" e os gateways "devem encaminhar a capacidade OAuth em `anthropic-beta`". **Medir** o tráfego de assinatura pelo proxy local é, portanto, uma configuração documentada.
- **Reescrever o modelo** das requisições de assinatura não é tratado por nenhum dos textos. Isso reduz o uso em vez de contornar limites, mas não é explicitamente permitido. Decisão: o roteamento fica em `observe` por padrão para todo login, e o `doctor` avisa quando `enforce` está ligado com login de assinatura, de modo que ligá-lo seja uma escolha informada do usuário.

## 2026-10-28 — O proxy pede `Accept-Encoding: identity` ao upstream

- **Contexto:** a API real comprime as respostas com gzip quando o cliente permite, e o proxy precisa ler o `usage` do corpo da resposta para medir os tokens com exatidão.
- **Decisão:** o proxy reescreve o `Accept-Encoding` para `identity` na requisição que envia ao upstream. É o único cabeçalho de requisição que ele reescreve; todos os outros são encaminhados como vieram.
- **Consequências:** as respostas chegam sem compressão, então o streaming e o `usage` podem ser lidos enquanto passam. Se um upstream comprimir mesmo assim, a resposta é encaminhada sem alteração e o uso dela não é registrado (fail-open: o Claude Code nunca é afetado, a medição fica com uma lacuna).

## 2026-10-28 — Como o proxy em segundo plano é ligado

- **Contexto:** `tokred proxy start` precisa deixar um proxy rodando depois que o comando retorna, e `proxy stop` e `tokred claude` precisam encontrá-lo e falar com ele com segurança.
- **Decisão:** o proxy em segundo plano é o mesmo binário, iniciado como `tokred proxy start --foreground` (o serviço de usuário executa o mesmo comando), então não há um segundo executável para manter sincronizado. O estado vive em `<data>/proxy/`: `secret` (só do dono, `0600` no Unix), que os endpoints administrativos (`/__tokred/...`) exigem, e `pid`. O `proxy stop` envia o pedido de desligamento e **espera até o processo sumir** (falha se o proxy não parou) antes de limpar o arquivo de pid.
- **Consequências:** `proxy stop && proxy start` é seguro para scripts; um arquivo de pid velho é ignorado quando o proxy não responde no endpoint de saúde.

## 2026-10-28 — O disjuntor da compressão sobrevive a uma reinicialização

- **Contexto:** quando requisições reescritas continuam sendo rejeitadas, o disjuntor desliga a compressão. Só em memória, uma reinicialização o ligaria de novo e as mesmas falhas se repetiriam.
- **Decisão:** o disparo é persistido na tabela `meta` do banco (`compression_tripped_at`). Enquanto estiver definido, o proxy sobe com a compressão em `observe` mesmo que o `tokred.toml` diga `enforce`; `tokred compression on` o limpa.
- **Consequências:** uma reinicialização (ou um reboot com o serviço de usuário) nunca religa em silêncio um recurso que estava falhando; `tokred compression status` mostra que o disjuntor disparou.

## 2026-10-28 — `tokred compression on|off` edita a configuração de usuário do Claude Code só quando executado

- **Contexto:** a compressão reversível precisa do servidor MCP `tokred_retrieve` registrado no Claude Code, o que significa editar sua configuração de usuário (`~/.claude.json`), um arquivo que guarda mais que os dados do tokred.
- **Decisão:** só `tokred compression on` (registra o servidor e define `[compression] mode = "enforce"`) e `off` (remove o registro e define `observe`) tocam nesse arquivo, e só quando você os executa. O conteúdo anterior é salvo ao lado como `.tokred.bak` antes. `configure`, `tokred claude` e o proxy nunca o editam.

## 2026-10-28 — Riscos conhecidos do proxy

- **A compactação parece um prompt simples.** As requisições de compactação não trazem cabeçalho que as distinga na versão do Claude Code testada, então uma poderia ser lida como prompt simples e alimentar uma sugestão de modelo mais barato. Sugestões só aconselham, e `routing.compaction` fica em `keep` por padrão, então o custo de uma leitura errada é uma sugestão errada.
- **O uso não é lido quando o upstream comprime mesmo assim** (veja a entrada do `Accept-Encoding` acima).

## 2026-10-28 — Windows: um `proxy start` com saída capturada espera o proxy

- **Contexto:** os testes de ponta a ponta do proxy travaram só no Windows: um proxy desacoplado herda os handles de pipe de um chamador que captura a saída, então o chamador nunca vê o fim da saída até o proxy sair.
- **Decisão:** no início, os testes subiam o proxy sem capturar a saída. Substituído no dia seguinte por iniciar o proxy por `Start-Process` (veja a entrada de 2026-10-29 abaixo). Os jobs de CI param após 20 minutos para que um travamento falhe com seu log.

## 2026-10-29 — Redes corporativas: túnel por proxy HTTP e autoridades certificadoras extras

- **Contexto:** o proxy conectava direto à API com as raízes da Mozilla embutidas, então falhava atrás de um proxy HTTP corporativo e atrás de um que inspeciona TLS com autoridade própria.
- **Decisão:** o cliente de upstream faz túnel com `CONNECT` pelo proxy indicado em `HTTPS_PROXY`, senão `ALL_PROXY`, senão `HTTP_PROXY` (e os nomes em minúsculas; `HTTP_PROXY` é um recurso de reserva porque muitas imagens corporativas só definem esse), respeitando o `NO_PROXY` com as regras do curl. Isso usa o `Tunnel` e o proxy matcher do `hyper-util` (a feature `client-proxy`; nenhum handshake escrito à mão). Só há suporte a proxies `http://` e só para um upstream `https` (um upstream `http` em loopback é sempre direto). A confiança é as raízes embutidas mais o repositório do sistema operacional (`rustls-native-certs`, só `ring`) mais os arquivos PEM de `[proxy] ca_bundle`, `NODE_EXTRA_CA_CERTS` e `SSL_CERT_FILE`, lidos com `rustls-pki-types` (não o `rustls-pemfile`, sem manutenção). Valores de proxy sem suporte (`socks5://`, `https://`) ou malformados, e arquivos de CA ausentes ou vazios, são registrados uma vez pelo nome da variável ou do arquivo (nunca o valor, que pode conter uma senha) e ignorados: o proxy conecta direto ou com as raízes restantes, então o tokred nunca quebra o Claude Code. Um proxy alcançável que recusa o túnel, ou um certificado não confiável, resulta no `502` de sempre. As configurações são lidas uma vez na partida, a partir do ambiente do processo que inicia o proxy.
- **Consequências:** redes que inspecionam TLS e que fazem túnel funcionam com as variáveis que o Claude Code já usa. Dependências novas: `rustls`, `rustls-native-certs`, `rustls-pki-types`, `webpki-roots` e, só para testes, `rcgen` e `tokio-rustls`; o `cargo deny` passa e não há `aws-lc` (ele exige `cmake` e NASM e quebra os builds musl e Windows). SOCKS e TLS até o proxy ficam fora do escopo. Mudanças de ambiente exigem reiniciar o proxy.

## 2026-10-29 — Windows: o proxy em segundo plano é iniciado por `Start-Process`

- **Contexto:** o proxy desacoplado herdava os pipes de um chamador que captura a saída (veja a entrada de 2026-10-28).
- **Decisão:** no Windows, `spawn_detached` executa `powershell.exe -NoProfile -NonInteractive -Command "Start-Process -FilePath <exe> -ArgumentList <args> -WindowStyle Hidden -PassThru | Select-Object -ExpandProperty Id"`, que inicia o proxy sem os handles deste processo, e lê o pid da primeira linha da saída (só da primeira linha, para que um pipe aberto perdido não segure a chamada). Caminhos e argumentos são entre aspas por funções puras testadas em todos os sistemas operacionais; um spawn desacoplado simples é o recurso de reserva quando o PowerShell não pode ser usado. Sem `unsafe` e sem dependência nova. O teste de ponta a ponta que executa `proxy start` com saída capturada roda em todos os sistemas operacionais e espera o comando em uma thread com prazo de 60 segundos, porque matar o comando não adianta enquanto o proxy ainda segura os pipes capturados.
- **Consequências:** `out=$(tokred proxy start)` retorna no Windows também. No Windows, a salvaguarda do `session-start` e o `initialize` ainda iniciam o proxy desacoplado sem streams padrão, e um filho pode herdar handles como o pipe de onde a saída de um hook é lida, então o chamador pode esperar o próprio timeout. Isso foi mantido: esperar é melhor que o Claude Code ficar trancado do lado de fora por um proxy morto, e o hook ainda sai com 0.

## 2026-10-29 — As requisições do proxy e as transcrições são unidas uma vez, antes de qualquer métrica

- **Contexto:** com o proxy rodando, toda requisição está em `proxy_requests` e nas transcrições (`usage`); somar as duas conta cada uma duas vezes, e usar só uma perde ou os dados exatos (modelo que respondeu, divisão do cache de 1 hora, latência) ou as requisições feitas enquanto o proxy estava desligado.
- **Decisão:** o `merge_usage` (puro, no `tokred-metrics`) roda uma vez, onde os dados são carregados, então `stats`, os dashboards de terminal, o relatório HTML, a projeção e o antes/depois leem uma só lista. Uma requisição do proxy é pareada com a linha da transcrição de mesmo `message_key`; só uma requisição do proxy **sem** chave recorre à mesma sessão, tokens iguais (os quatro contadores, não a divisão de 1 hora) e um horário dentro de 5 segundos da requisição ou até 5 segundos depois do fim da resposta (a linha da transcrição é escrita quando a resposta termina, então um simples "dentro de 5 s da chegada" perderia qualquer resposta com mais de 5 s). O pareamento é um para um e do mais próximo no tempo primeiro, então duas requisições idênticas nos mesmos segundos nunca são fundidas. Um par fica com os números do proxy e com o projeto e a chave da transcrição; linhas sem par dos dois tipos permanecem. Uma requisição respondida com status não 2xx ou sem tokens não é uma mensagem. Duas linhas do proxy com o mesmo id, ou a mesma chave de mensagem, contam uma vez. As linhas de `usage` ganham `origin` e `agent_type` opcionais (nunca guardados) para que `--by origin` e `--by agent-type` não precisem de um segundo caminho de dados; sem o proxy, `origin` recorre a `main` ou `subagent` a partir das transcrições.
- **Consequências:** uma requisição passada pelo proxy é contada uma vez qualquer que seja a ordem de ingestão; a economia de roteamento precifica o modelo que realmente respondeu; sem linhas do proxy, as linhas da transcrição passam intactas. Uma chave presente nos dois lados mas diferente são duas requisições (nunca unidas por tempo). Coberto por testes unitários, um snapshot e propriedades (nada se perde, nada se repete, unir de novo não muda nada).

## 2026-10-29 — O `stats` mostra a sugestão pendente e nunca a marca como mostrada

- **Contexto:** a status line deliberadamente nunca marca uma sugestão como mostrada (ela é redesenhada o tempo todo), para não escondê-la da mensagem de uma linha; o `stats --json` também não deve gravar no banco.
- **Decisão:** o `stats` mostra a sugestão mais recente do período que ainda não foi mostrada, com um ponteiro para `tokred suggest`, e não altera o marcador; só o canal de mensagem o define.
- **Consequências:** o `stats` é somente leitura e repetível; a mesma sugestão continua no `stats` até o canal de mensagem mostrá-la.

## 2026-10-29 — As seções do proxy no relatório HTML só aparecem quando o proxy registrou algo

- **Decisão:** as seções do proxy (roteamento de modelo, sobrecarga, falhas de cache por causa, custo por origem e tipo de agente) só são renderizadas quando o relatório tem dados do proxy, e os dados só são embutidos na página nesse caso. Dentro delas, uma parte vazia (sem falhas de cache, sem decisões) é omitida em vez de exibida vazia.
- **Consequências:** um relatório sem o proxy é byte a byte o que era antes; o aviso "ainda sem dados" vale para as seções originais.

## 2026-10-29 — Uma função monta o relatório a partir dos repositórios

- **Contexto:** o proxy serve o mesmo relatório em `/metrics`, e o `tokred-proxy` não pode depender dos crates de store nem de relatório.
- **Decisão:** `report_html(repos, prices, now, since, audit)` no `tokred-cli` recebe as portas de repositório (uso, requisições do proxy, eventos de hook, roteamento, sugestões, meta) e devolve o HTML como string: sem ingestão de transcrições, sem acesso a arquivos ou à rede, sem sincronização de preços. O `tokred metrics` ingere as transcrições e depois grava o que `report_html` devolve; o proxy recebe a mesma função pela porta `DashboardSource` do `tokred-core`.
- **Consequências:** o arquivo e a página não podem divergir.

## 2026-10-29 — Registros de requisição do proxy mais ricos

- **Contexto:** `proxy_requests` não tinha o motivo de parada, os nomes das ferramentas nem a duração do cache, e não dizia sob qual modo de roteamento a requisição rodou.
- **Decisão:** quatro colunas (`stop_reason`, `tools` como texto JSON só com nomes e no máximo 200, `cache_ttl` como `5m`, `1h` ou nulo, `routing_mode`); linhas antigas mantêm valores nulos ou vazios. `stop_reason` e os nomes das ferramentas já eram lidos, então nada novo é lido no caminho da requisição. O `cache_ttl` é calculado depois da resposta a partir do corpo enviado, olhando só os lugares em que o Claude Code põe pontos de quebra (nível superior, `system`, `tools`, blocos de conteúdo das mensagens); um `cache_control` sem `ttl` conta como `5m`, e o mais longo vence. O `routing_mode` vem do modo de `[routing]` com que o proxy foi montado, não das notas dos hooks: as notas são zeradas quando um hook falha, e essas são exatamente as requisições que vale explicar.
- **Consequências:** o `tokred-store` continua sem precisar de uma biblioteca JSON: os nomes das ferramentas são serializados no `tokred-core`.

## 2026-10-29 — `retention_days` limpa todas as tabelas do proxy, em um agendamento

- **Contexto:** nada apagava os dados do proxy, embora `[proxy] retention_days` existisse.
- **Decisão:** quando o proxy sobe, e depois a cada hora, linhas mais antigas que `retention_days` são apagadas de `proxy_requests`, `routing_decisions`, `prompt_summaries`, `suggestions`, `shadow_savings`, `stored_results` e `compressed_blocks`, em uma thread própria (o trabalho de banco nunca roda no caminho da requisição). `0` guarda tudo, e essa regra vive na função de limpeza porque o antigo `prune_proxy_requests(now, 0)` significa "apagar tudo antes de agora". Os resultados de ferramentas guardados e as substituições de uma conversa que ainda tem uma requisição dentro do período são mantidos qualquer que seja a idade: um resultado de ferramenta substituído precisa ser enviado com os mesmos bytes a cada volta (mudá-lo quebra o cache de prompt) e o modelo ainda pode pedir o original. Registros de uma requisição em andamento nunca correm risco: são gravados quando a resposta termina, com o horário em que a requisição chegou.
- **Consequências:** uma conversa longa mantém seus dados de compressão por mais tempo que `retention_days`; o custo é limitado pelo número de conversas ativas.

## 2026-10-29 — `[proxy] enabled` é a chave única; `tokred initialize` reconcilia as configurações do Claude Code

- **Contexto:** com a URL do proxy gravada nas configurações do Claude Code e o proxy fora do ar, o Claude Code não alcança a API. O instalador coloca um wrapper de shell `claude` que pode executar um comando antes de cada abertura.
- **Decisão:** `enabled` é a única chave voltada ao usuário. `false`: `tokred claude` executa o `claude` direto (uma linha no stderr), `tokred initialize` remove a URL do tokred do `settings.json`, `proxy start` e `proxy service install` recusam, o `doctor` mostra "disabled"; `proxy stop` e `status` continuam funcionando e um proxy em execução nunca é parado. `true`: o `initialize` sobe o proxy quando ele não está rodando e, depois que responde, define `env.ANTHROPIC_BASE_URL`; se o proxy não puder ser iniciado, a URL é removida de novo (ela só é gravada depois que o proxy responde, então não há janela com uma URL morta) e um aviso é impresso. `tokred proxy enable` e `disable` editam o `tokred.toml` e executam a mesma reconciliação na hora; o `unconfigure` não mexe no `tokred.toml`.
- **Sem `persist_base_url`.** Uma opção anterior para persistir a URL foi removida: com o `enabled` mantendo a URL no `settings.json`, aberturas a partir de uma IDE que nunca passam pelo wrapper já a usam, então a chave só acrescentava um segundo jeito de chegar ao mesmo estado. Arquivos que ainda têm a chave continuam funcionando (ela é ignorada). O risco é coberto por três salvaguardas que dependem do `enabled`: o `initialize` nunca deixa uma URL apontando para um proxy que não conseguiu iniciar; o hook `session-start` sobe um proxy morto; o `doctor` avisa.
- **Um valor que não é do tokred nunca é tocado.** "A URL do tokred" é a URL atual do proxy ou uma URL que o tokred gravou antes, registrada uma por linha em `<data>/proxy/base_urls` (um arquivo simples, para que o caminho de cada abertura não abra o banco, e para que uma mudança de porta seja acompanhada). O valor exportado no shell também conta como valor atual. Outro gateway, mesmo em `127.0.0.1`, faz o `initialize` imprimir um aviso e não fazer nada (e não iniciar nada). A edição do arquivo passa pelo `tokred-install` (funções de merge puras com fixtures e testes de propriedade): ela altera só essa chave, mantém sua posição e as outras chaves de `env`, não grava nada quando nada muda e não faz backup (o backup que o `configure` fez precisa guardar o original do usuário).
- **O `tokred initialize` é fail-open e silencioso.** Sempre sai com 0 (argumentos inválidos à parte), não imprime nada a menos que haja um aviso (`--verbose` e `--json` dizem mais), lê um arquivo e sonda o proxy uma vez, e não abre o banco. Ele usa a pasta do Claude Code que o próprio `claude` usaria: `$CLAUDE_CONFIG_DIR`, senão a pasta salva pelo `configure`, senão `~/.claude`.
- **A salvaguarda do `session-start`** é composta no binário, depois que o hook respondeu, e não no `tokred-hooks` (que não tem código de rede): quando `enabled` é true, a `ANTHROPIC_BASE_URL` da sessão (ambiente, senão `settings.json`) é do tokred e o proxy não responde a uma sondagem de 200 ms, ele inicia o proxy desacoplado e retorna sem esperar. Ela depende do `enabled`, então o benchmark de hooks e os testes de hooks nunca iniciam um proxy.
- **O serviço de usuário.** Enquanto desabilitado, `proxy start --foreground` recusa, então um serviço instalado falharia e seria reiniciado pelo gerenciador de serviços: `proxy service install` recusa enquanto desabilitado e `proxy disable` manda executar `proxy service uninstall` quando há um serviço instalado.
- **Consequências:** `tokred proxy disable` é a saída de qualquer problema, e apagar a chave à mão também.

## 2026-10-29 — Instalação: scripts e arquivos do release, sem gerenciadores de pacotes

- **Contexto:** o primeiro desenho distribuía o tokred pelo Homebrew, Scoop, winget e crates.io, além dos instaladores do `dist`. Isso significava quatro tokens, dois repositórios extras, um primeiro manifesto do winget manual e um workflow customizado que nunca tinha rodado.
- **Decisão:** o release do GitHub é o único canal. `install/install.sh` e `install/install.ps1` baixam `tokred-<target>` e seu `.sha256` do release (`releases/latest/download/`, ou `releases/download/v$TOKRED_VERSION/`; `TOKRED_RELEASE_BASE_URL` substitui a base e aceita `file://`, o que torna os scripts testáveis offline), verificam o digest, instalam em `$TOKRED_HOME/bin` (`~/.tokred/bin`, Windows `%LOCALAPPDATA%\tokred\bin`), adicionam essa pasta ao `PATH` e executam `tokred configure`. O `dist-workspace.toml` tem `installers = []` e uma entrada `extra-artifacts` que envia os quatro scripts junto dos arquivos; o workflow de gerenciadores de pacotes e o tap do Homebrew foram apagados, e o release não precisa de segredo. Os mesmos scripts estão na `main` para uso antes de existir um release.
- **Desenho:** os scripts são finos. Tudo que é interativo (porta, gravação de conteúdo, confirmações) é Rust, testado por unidade pela costura `Io`; os scripts só baixam, verificam, copiam, editam `PATH` e perfis e chamam o binário. Com `curl | sh` o stdin é o pipe, então o script dá ao binário o terminal (`</dev/tty`) quando há um e `--yes` caso contrário. Sem `sudo`, nada gravado fora de `TOKRED_HOME` e dos arquivos de perfil, root recusado sem `--yes`. Os downloads do próprio script de shell usam `--proto '=https'` para `https://` e uma cópia simples para `file://`. Os arquivos `.sh` são LF em todo checkout (`.gitattributes`).
- **Ressalvas do PowerShell:** `irm | iex` não consegue passar parâmetros, então cada opção também tem uma variável de ambiente. O `PATH` de usuário é alterado com `[Environment]::SetEnvironmentVariable(..., 'User')`, que expande `%VARS%` das entradas existentes ao gravá-las de volta; aceitou-se a chamada mais simples. Os scripts ficam em ASCII, usam `throw` em vez de `exit` (que fecharia uma janela interativa sob `iex`) e renomeiam um `tokred.exe` em execução para `.old` em vez de sobrescrevê-lo. Eles são exercitados pelo `install_test.ps1` no job de CI do Windows.
- **Consequências:** instalar e configurar são um só passo: o script executa o `configure` (veja a entrada do `configure` abaixo para o que o passo do proxy virou). O `tokred update` para instalações por gerenciador de pacotes acabou (veja abaixo).

## 2026-10-29 — `configure` pergunta a porta do proxy e a gravação de conteúdo; `uninstall` faz perguntas

Substituída no mesmo dia: as perguntas passaram para o `tokred proxy enable`, e o `uninstall` não pergunta nada. Veja "`configure` é só hooks e agentes" e "`uninstall` remove tudo e não pergunta nada" abaixo.

## 2026-10-29 — `tokred update` só substitui o binário do próprio script

- **Decisão:** `InstallMethod` é `Script | Other`. Um binário cuja pasta é igual a `$TOKRED_HOME/bin` (resolvida com `canonicalize` nos dois lados, para que `/var` versus `/private/var` do macOS não importe) se atualiza com o download, o checksum e a troca atômica já existentes (um `.exe` no Windows é renomeado para `.old` antes); qualquer outro (compilado do código-fonte, copiado à mão, sobra de um canal antigo) recebe o aviso "reinstale com o script de instalação". A verificação diária de versão não muda e usa a mesma dica.
- **Consequência:** o padrão se inverteu. Um local desconhecido era tratado como instalação por script e se autoatualizava; agora não é tocado. Essa é a falha mais segura: o tokred nunca sobrescreve um binário que não colocou ali.

## 2026-10-29 — O wrapper `claude`

- **Contexto:** a chave do proxy precisa valer em todo terminal, e o Claude Code lê a `ANTHROPIC_BASE_URL` só na partida. O `tokred initialize` reconcilia as configurações do Claude Code com a chave; algo precisa executá-lo antes do `claude`.
- **Decisão:** o script de instalação define uma **função** de shell `claude` no mesmo bloco marcado da linha do `PATH`: `claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }`. Uma função (não um alias) repassa argumentos e não pode recursar; `command claude` é o desvio. Ela é fail-open: sem o `tokred`, o `claude` inicia como antes. O bloco é o mesmo texto em todo perfil POSIX (`~/.profile`, `~/.bashrc`, `~/.bash_profile`, `~/.zshrc`, e um `.zshrc` é criado quando o shell de login é o zsh), precedido por `unalias claude` porque o instalador do próprio Claude Code pode adicionar um alias ao `.zshrc`, o que tornaria a definição da função um erro de sintaxe no zsh. O fish recebe um arquivo próprio, `conf.d/tokred.fish`, com a função equivalente (apagado na desinstalação só quando contém nada além do nosso bloco); o Windows recebe um bloco marcado no `profile.ps1` do Windows PowerShell e, quando presente, do pwsh, chamando o binário real por `Get-Command claude -CommandType Application`. O `cmd.exe` não tem aliases persistentes e não é coberto.
- **Chaves:** `--no-alias` / `TOKRED_NO_ALIAS=1` o pulam e são independentes de `--no-modify-path`; o bloco é reescrito a cada execução (então um bloco antigo ganha o wrapper), nunca pulado quando presente, e gravado com `cat >` para que dotfiles que são symlinks continuem symlinks.
- **Limite:** se o `claude` real só for alcançável por um alias, o `command claude` não o encontra.

## 2026-10-29 — O dashboard: o balanço de privacidade, deltas por volta, login e desligamento

- **Contexto:** o proxy serve um dashboard ao vivo (`/dashboard`, `/metrics`) que mostra o que foi enviado ao Claude e o que voltou. A promessa anterior de que o conteúdo das mensagens nunca é guardado não se sustenta se isso é mostrado, e o Claude Code reenvia a conversa inteira a cada requisição.
- **Decisão (privacidade):** os conteúdos só são capturados com `[proxy] log_bodies = true`, desligado por padrão. Desligado significa que nada sobre conteúdos é lido de requisições ou respostas, mantido em memória, enviado ao navegador ou guardado (o evento `turn_delta` também é conteúdo e não é enviado); o dashboard ainda mostra metadados (modelos, tokens, custo, tempos, status, motivo de parada, nomes das ferramentas, roteamento, subagentes). Ligado significa **deltas** por volta, não corpos: as mensagens novas desde a requisição anterior da mesma sessão e subagente (rastreadas por um hash por mensagem, com uma checagem rápida da primeira e da última mensagem; depois de uma compactação ou retrocesso, as mensagens que diferem são enviadas) e a resposta remontada, cada texto truncado em 20.000 caracteres. Cabeçalhos e credenciais nunca são capturados. Os conteúdos ficam em `captured_turns` e são limpos na partida e a cada hora: só sessões ativas ou entre as últimas 10 os mantêm, e nada passa de `retention_days`. O motivo de parada e os nomes das ferramentas chamadas são metadados, então `captured_turns` tem uma linha para cada volta e só as colunas de conteúdo dependem do `log_bodies`; a limpeza esvazia os conteúdos e mantém a linha até `retention_days`.
- **Decisão (login):** outros usuários locais alcançam o loopback e um navegador não consegue enviar `x-tokred-secret`, então `POST /__tokred/dashboard-token` (com o segredo) emite um token válido por 60 segundos e uma vez; `GET /dashboard?token=` o troca por um cookie aleatório (`HttpOnly`, `SameSite=Strict`, `Path=/` para que o `/metrics` o compartilhe) e redireciona. Tokens e cookies são comparados em tempo constante contra todos os valores guardados, e vivem em memória (uma reinicialização desconecta você). A aleatoriedade vem de `RandomState`, como o segredo administrativo, para evitar uma dependência nova. Requisições com `Sec-Fetch-Site: cross-site` ou `same-site` são recusadas (`same-site` também: outro aplicativo web local em outra porta é o mesmo site), nenhum cabeçalho CORS é enviado e toda página leva uma CSP estrita. O `/metrics` tem a própria CSP (`style-src 'unsafe-inline'`) e permite exatamente os scripts inline do próprio relatório pelo hash SHA-256, para que o seletor de período e as dicas continuem funcionando enquanto nada mais pode executar.
- **Decisão (eventos e união):** um canal de broadcast limitado de eventos já enquadrados; nada é construído quando ninguém escuta, um assinante lento perde os eventos mais antigos (ele relê a API ao reconectar; deltas nunca são reenviados). O proxy mantém em memória as voltas em andamento e as últimas encerradas e as une por id de requisição sobre o que o store devolve, com a versão ao vivo vencendo, então uma sessão mostra as duas. Subagentes em execução são rastreados a partir do id de agente das requisições em andamento (em execução enquanto uma está em andamento e por 60 segundos depois da última); uma tarefa de varredura os encerra e para junto com o proxy.
- **Decisão (desligamento):** o hyper espera um corpo em streaming em andamento quando uma conexão é mandada parar, então um stream de eventos aberto seguraria o proxy até o timeout de drenagem (10 s, mais que o `proxy stop` espera). A tarefa do stream de eventos faz select no sinal de desligamento, então streams abertos terminam na hora; um teste de ponta a ponta abre `/dashboard/events`, executa `proxy stop` e confere que o stream termina e o processo sai.
- **Consequências:** ligar o `log_bodies` guarda texto de conversa no banco local; os dados nunca saem da máquina e são limpos, mas quem puder ler o banco os lê. Uma requisição de compactação aparece como `auxiliary`. Só as 500 voltas mais novas de uma sessão são devolvidas (`truncated: true`). Requisições sem id de sessão não são acompanhadas. O relatório `/metrics` é gerado a partir das transcrições a cada requisição, em uma tarefa bloqueante, sem a auditoria de projeto.

## 2026-10-29 — `uninstall` remove tudo e não pergunta nada; `unconfigure` não toca no proxy

- **Contexto:** o primeiro `tokred uninstall` fazia perguntas, mantinha a pasta de dados por padrão e deixava o binário, a entrada do `PATH` e o wrapper `claude` para o script de desinstalação. O `tokred unconfigure` também removia a `ANTHROPIC_BASE_URL` do proxy, o que o tornava um segundo dono de uma configuração que pertence ao proxy: o wrapper `claude` recolocava a URL na abertura seguinte, e um proxy que devia continuar rodando perdia seu lugar no caminho.
- **Decisão:** `tokred uninstall` é a remoção completa e não pergunta nada: executa, em ordem, `proxy stop`, `proxy service uninstall` (só quando o arquivo do serviço existe para esta pasta home, para que uma execução em sandbox nunca alcance o `launchctl` ou o `systemctl` reais), `compression off` (só quando o servidor MCP está registrado) e `unconfigure`, apaga a URL do proxy e a pasta de dados, depois o binário, sua pasta, a entrada do `PATH` e o wrapper. Cada passo é de melhor esforço (uma falha é impressa e o passo seguinte roda, então um proxy quebrado nunca deixa hooks para trás), exceto que, se algum passo falhar, o binário é mantido, para que o Claude Code nunca aponte para um binário que não existe mais. A última parte é o script de desinstalação embutido no binário (`--cleanup-only`), então a lógica existe uma vez e os testes do instalador no CI a cobrem em todos os sistemas; no Windows ele é iniciado desacoplado e espera o processo sair, porque um executável em execução não consegue se apagar. O binário só é apagado quando o script de instalação o colocou em sua pasta. O `unconfigure` remove só hooks e agentes; só `tokred proxy enable` / `disable` e `uninstall` alteram a URL.
- **Consequências:** `--yes` e `--purge` do `uninstall` são aceitos e ignorados; os scripts de desinstalação não aceitam opções. Os testes de ponta a ponta do `uninstall` rodam só no Linux e macOS, porque no Windows o script tocaria os perfis reais do PowerShell da máquina (o teste do CI do Windows faz backup deles e os restaura).

## 2026-10-29 — `configure` é só hooks e agentes; o proxy é preparado por `tokred proxy enable`

- **Contexto:** o `configure` tinha ganhado três tarefas do proxy: gravava `[proxy] enabled = true` em uma instalação nova, perguntava a porta do proxy e perguntava se o conteúdo das conversas pode ser gravado. O proxy é opcional, então um comando sobre hooks e agentes estava decidindo coisas sobre um servidor que o usuário pode nunca querer, e o `unconfigure` não conseguia espelhá-lo.
- **Decisão (a regra de separação):** `tokred configure` e `tokred unconfigure` tocam só em hooks e agentes, mais o que é neutro (o banco e o `tokred.toml` padrão). Tudo sobre o proxy vive nos comandos do proxy. O `configure` não grava nenhum valor de `[proxy]` além dos padrões do merge de configuração simples (`enabled = false` continua sendo o padrão do código).
- **`tokred proxy enable [--port <n>] [--record-content <true|false>] [--yes]`** assume as perguntas. As duas flags recebem um valor (`yes`, `no`, `1`, `0` também funcionam, em qualquer caixa); `TOKRED_PORT` e `TOKRED_RECORD_CONTENT` são o recurso de reserva não interativo e uma flag vale mais que a variável. Em um terminal, um valor não informado é perguntado: a porta é aberta em `127.0.0.1` para ver se está livre, uma porta que o proxy desta instalação já serve conta como livre, e o padrão é a porta salva, senão 1029. Sem terminal, ou com `--yes`, só o que uma flag ou variável diz é gravado; o resto mantém o valor salvo, senão porta 1029 e conteúdo não gravado (privacidade em primeiro lugar), e uma porta ocupada informada por flag só avisa. Ele grava `[proxy] bind`, `log_bodies` e `enabled = true` no lugar (as outras chaves do usuário permanecem) e executa a reconciliação. Um proxy já rodando em outra porta, ou com outra configuração de gravação, é reiniciado para que a mudança valha. O `--yes` existe porque o script de instalação precisa poder dizer "ninguém pode responder" mesmo quando o stdin é um terminal (o PowerShell não consegue redirecioná-lo como o `</dev/null` faz).
- **O padrão da pergunta de conteúdo.** O `configure` sempre deixa `log_bodies = false` no arquivo (é um padrão do merge), então um valor salvo sempre existiria e um primeiro `enable` ofereceria "não". Um `false` salvo, portanto, só conta como escolha depois que o proxy foi habilitado (ou quando é `true`); antes disso o padrão é sim.
- **Scripts de instalação:** depois do `configure`, `install.sh` e `install.ps1` executam `tokred proxy enable`, repassando `--port` (flag do script ou `TOKRED_PORT`). `--no-proxy` (`-NoProxy`, `TOKRED_NO_PROXY=1`) pula o passo: só hooks e agentes. `--no-configure` pula os dois, e um `configure` que falhou também. Um `configure` cujo plano o usuário recusa sai com código 2, como o `uninstall`: os scripts leem 2 como "pare, não mude mais nada", pulam o passo do proxy e não imprimem aviso, então recusar deixa a máquina intacta. Os testes do instalador executam toda instalação com `--no-proxy`, exceto os dedicados, que param o proxy que iniciam.
- **Consequências:** `tokred configure --yes` em uma máquina com o proxy ligado não muda nada nele, então a reexecução silenciosa depois de uma atualização e o `update` nunca tocam no proxy. Quem executa só o `configure` tem hooks e agentes e nenhum proxy até `tokred proxy enable`; o `doctor` mostra "disabled" nesse estado.

## 2026-10-29 — `configure` e `unconfigure` são a chave de hooks e agentes; não existe comando `integration`

- **Contexto:** foi proposto um comando separado de liga/desliga para hooks e agentes, com um estado "desligado" lembrado para que uma atualização não reinstalasse o que o usuário desligou. O estado lembrado já existia de forma quebrada: o `unconfigure` deixava no banco o marcador "configurado pela versão X", então o primeiro comando de um binário mais novo reaplicava a configuração e recolocava em silêncio os hooks e agentes que o usuário tinha removido.
- **Decisão:** nenhum comando novo. `tokred unconfigure` é o desligar durável (remove hooks e agentes, mantém agentes que você editou e limpa o marcador) e `tokred configure` é o ligar (instala a versão do binário que está rodando). O proxy tem o próprio par, `proxy enable` / `disable`, e os três são independentes.
- **Consequências:** nada para guardar enquanto desligado: o binário é a única fonte das definições de hooks e dos templates de agentes, então uma atualização enquanto desligado não precisa de renovação. Um marcador vazio significa "não configurado" (`needs_reconfigure` o ignora), a mesma convenção da marca do disjuntor da compressão.

## 2026-10-29 — Sem build nativo para Windows ARM

- **Contexto:** a primeira execução de release compilou cinco alvos e falhou em `aarch64-pc-windows-msvc`: o script de build do crate `ring` executou o compilador C com uma opção que ele não aceita (`/imsvc`), e um alvo que falha impede que o release inteiro seja publicado.
- **Decisão:** o alvo foi removido do `dist-workspace.toml`; o `install.ps1` instala o build x86_64 no Windows em ARM, que o Windows executa por sua emulação. Um build nativo pode voltar quando o `ring` (ou um provedor em Rust puro) compilar lá.
- **Consequências:** cinco arquivos por release; usuários de Windows em ARM executam um build emulado.

## 2026-10-29 — Um smoke test do release publicado, em runners nativos e containers Linux

- **Contexto:** os testes do instalador montam um release falso a partir do binário recém-compilado e o servem do disco, e os testes em Rust usam o mesmo binário. Nenhum prova que o que foi realmente publicado funciona: o arquivo e seu checksum, os scripts de instalação que acompanham a tag, o binário musl estático em uma máquina sem glibc, os scripts de desinstalação dessa tag.
- **Decisão:** `tests/release/smoke.sh` e `smoke.ps1` instalam a partir do release real do GitHub de uma tag (o script de instalação do próprio release, nada definido em `TOKRED_RELEASE_BASE_URL`) e conferem o estado real em todo o produto (o que ele confere está listado em [release](release.pt-BR.md)). O workflow `Release smoke` o executa quando o `Release` termina com sucesso para uma tag `v*`, ou manualmente para uma tag. Ele roda em runners nativos (Ubuntu, macOS arm64, macOS Intel, Windows) porque os instaladores diferem por plataforma e um runner hospedado é o mais próximo da máquina de um usuário, e em containers `alpine:3.20`, `debian:12-slim` e `fedora:40`: o Alpine não tem glibc, então uma execução verde prova o binário musl estático. Os scripts vêm da branch em que o workflow roda, os arquivos vêm da tag, então uma tag cortada antes de o teste existir pode ser conferida. Um release não é anunciado até o workflow ficar verde para a tag.
- **Consequências:** um job que falha é um achado sobre os artefatos publicados, não sobre a árvore de trabalho; a correção é um novo release candidate. A primeira execução encontrou dois problemas do produto: `proxy enable --record-content false` em um proxy em execução mudava o arquivo mas não o proxy em execução (um proxy em execução agora é reiniciado), e o `tokred update` imprimia um 404 cru em vez de dizer que não existe release estável. O que sobra depois de uma desinstalação é conferido e impresso como notas, não escondido. O script do Windows não consegue isolar o `PATH` de usuário nem os perfis do PowerShell, então faz backup e restaura, e o job do macOS Intel usa o rótulo `macos-15-intel` porque o `macos-13` foi aposentado.
- **O que ele não consegue cobrir:** o próprio Claude Code com um login real (tráfego de assinatura, saída real do modelo, a inicialização do Claude Code), o ramo do `tokred update` que substitui o binário e um release que ainda não foi publicado.
