[English](decisions.md) | **Português**

# Registro de decisões

Formato: data, contexto, decisão, consequências. Onde a spec diverge da documentação oficial do Claude Code, a documentação vence e a divergência fica registrada aqui.

## 2026-10-28 — A ferramenta de subagentes é `Agent`, matcher `Agent|Task`

- **Contexto:** a spec manda confirmar se a ferramenta de subagentes é `Task` ou `Agent` (seção 7.2).
- **Decisão:** segundo a documentação de sub-agents a ferramenta é `Agent`, renomeada de `Task` no Claude Code v2.1.63, com `Task(...)` ainda aceito como alias. O hook `enforce` usa o matcher `Agent|Task` (a sintaxe de matcher aceita nomes exatos separados por `|`).
- **Consequências:** funciona em versões antigas e novas do Claude Code.

## 2026-10-28 — O `timeout` dos hooks é em segundos

- **Contexto:** a documentação de hooks define `timeout` em segundos (padrão 600 para hooks de comando).
- **Decisão:** todo hook do tokred é registrado com `timeout: 5`, como no exemplo da spec.

## 2026-10-28 — Comandos de hook do tokred são reconhecidos estruturalmente, não pela substring `tokred hook `

- **Contexto:** a seção 5.1 da spec diz que o `command` de uma entrada do tokred "contém o marcador `tokred hook `", mas o próprio exemplo é `"/opt/homebrew/bin/tokred" hook guard-read`: a aspa de fechamento fica entre `tokred` e `hook`, então essa substring nunca aparece nas entradas que o próprio tokred grava. Uma implementação literal jamais encontraria (e portanto jamais substituiria ou removeria) as próprias entradas.
- **Decisão:** um comando é do tokred quando executa um executável cujo nome de arquivo começa com `tokred` (`tokred`, `tokred.exe`, ou uma cópia com versão/renomeada como `tokred-0.4.0`), opcionalmente entre aspas, seguido de ` hook <nome>`. Descoberto por um teste unitário: sob `cargo test` o binário se chama `tokred-<hash>`, e uma regra de nome exato fazia o `configure` duplicar entradas e o `unconfigure` não remover nada.
- **Consequências:** idempotência e unconfigure funcionam com paths entre aspas, paths do Windows com espaço, binários renomeados e `tokred hook x` solto. Um wrapper como `bash -c 'tokred hook x'` deliberadamente não é reconhecido.

## 2026-10-28 — Os rótulos do formato de retorno ficam em inglês

- **Contexto:** o formato de retorno da spec usa `RESULTADO/ARQUIVOS/PENDÊNCIAS`, mas a seção 0 exige templates de agentes em inglês.
- **Decisão:** os rótulos são `RESULT`, `FILES` e `PENDING`.

## 2026-10-28 — Nomes dos arquivos de documentação em minúsculas

- **Contexto:** a spec diz `docs/DECISIONS.md` na seção 0 e `docs/decisions.md` na seção 16.2.
- **Decisão:** minúsculas, com o par `.pt-BR.md`. A tradução da spec (`docs/spec.md`) mantém o texto original.

## 2026-10-28 — Hooks são stubs fail-open na F0

- **Contexto:** a F0 registra os hooks da seção 7, mas a lógica deles chega nas fases F3 a F5; quem configurar no meio do caminho nunca pode ser prejudicado.
- **Decisão:** `tokred hook <evento>` consome o stdin e não imprime nada. O `hook` é despachado **antes** do parse de argumentos e sempre sai com 0, mesmo para evento desconhecido, porque o Claude Code trata o código 2 como "bloquear".

## 2026-10-28 — A versão mínima do Rust é 1.89

- **Contexto:** a edição 2024 exige 1.85, mas o `rusqlite` 0.41 declara 1.89.
- **Decisão:** `rust-version = "1.89"`, verificado com `cargo +1.89 check --locked` localmente e no CI.

## 2026-10-28 — Pasta de dados via `BaseDirs`, não `ProjectDirs`

- **Contexto:** a spec lista as pastas de dados por SO e cita `directories::ProjectDirs`. No Windows o `ProjectDirs` acrescenta `\data` (`%APPDATA%\tokred\data`), o que não bate com o `%APPDATA%\tokred` da spec.
- **Decisão:** usar `BaseDirs::data_dir().join("tokred")`, que dá exatamente os três paths da spec.

## 2026-10-28 — `option-ext` (MPL-2.0) permitido por exceção por crate

- **Contexto:** `directories` → `dirs-sys` → `option-ext`, licenciado MPL-2.0, que não está na lista de permitidas da spec (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib).
- **Decisão:** manter o `directories` (escolha da spec) e adicionar uma exceção do `cargo-deny` só para esse crate. A MPL-2.0 é copyleft por arquivo e o crate é usado sem modificação.

## 2026-10-28 — Remote do git mantido como o alias SSH existente

- **Contexto:** a spec diz `git@github.com:sostenes198/tokred.git`; o repositório já tinha `git@github.com-pessoal:sostenes198/tokred.git`.
- **Decisão:** manter o remote existente (escolha do autor: usar a configuração de SSH da máquina dele).

## 2026-10-28 — Detalhes de `configure` / `unconfigure` que a spec não fixa

- `--claude-dir` é aceito por `configure`, `unconfigure` e `doctor` (a spec o lista só para `configure`); um path explícito porém inválido é erro e não cai para os outros candidatos.
- `configure` também aceita `--force` (a spec o cita para o `update`).
- Um `settings.json` em branco vale `{}`; JSON inválido aborta sem escrever nada, sem backup e sem criar a pasta de dados.
- O `settings.json` só é regravado quando o resultado do merge difere, e é re-serializado com indentação de 2 espaços (a ordem das chaves é preservada, a formatação original não). O backup guarda o texto anterior exato.
- Limite do round-trip: um array de evento ou objeto `hooks` que **já estava vazio** e no qual o tokred fez merge é removido pelo `unconfigure`, porque "esvaziado pela nossa remoção" não se distingue de "vazio desde o início". Contêineres vazios que o tokred não tocou são preservados.
- Se o `settings.json` não existia, o `unconfigure` deixa `{}`.
- O `unconfigure` mantém os agentes que você editou (e, quando o banco não existe mais, todos os agentes, pois a origem deles não pode ser verificada).
- `--purge` se recusa a apagar uma pasta que não tenha `tokred.db` nem `tokred.toml`.
- O banco só é criado depois que o plano é confirmado.
- O `tokred.toml` só é regravado (sem comentários) quando faltavam chaves; caso contrário fica byte a byte igual.
- O `unconfigure` é tolerante: remove os handlers do tokred e deixa exatamente como está tudo que não entende (um `hooks` que não é objeto, um evento que não é array). Só o `configure`, que precisa escrever nesses lugares, reporta formato inválido, e apenas para os eventos em que escreve.
- O I/O de terminal passa por uma pequena abstração `Io` (flag interativa, entrada, saída), e a saída dos comandos é montada por funções puras em `String`. Isso permite testar prompts, o texto do plano e todos os estados do `doctor` sem TTY, e mantém a cobertura de linhas em 100%.
- `doctor --bench`, sincronização de preços e `tokred metrics` não fazem parte da F0 e ficam fora do início rápido até suas fases.

## 2026-10-28 — F6 (resumos de artefatos) e F7 (cache de respostas / proxy) adiadas até haver dados

- **Contexto:** a spec condiciona as duas fases a medições: F6 "só depois de medir F1 a F5" e F7 "decisão registrada em DECISIONS.md com base nos dados das métricas". O cache em camadas em si (L1/L2/L3, `cache stats`, `cache prune`, contagem de arquivos por raiz) faz parte de F1 a F5 e está implementado. O que seria novo na F6 é guardar *resumos de artefatos por hash de conteúdo* e servi-los no lugar do artefato, o que exige uma forma de produzir os resumos (o tokred não chama modelo; teria de capturar a saída do subagente `tokred-summarizer`) e um hook para servi-los. Ainda não há dados de uso que digam se isso compensa a complexidade.
- **Decisão:** não construir F6 nem F7 ainda. Coletar dados com as ferramentas entregues e decidir com eles.
- **Critérios para seguir com a F6** (conferir com `tokred stats` e `tokred metrics` após cerca de quatro semanas de uso real): as negações `large_file` mais `reread` são uma fatia relevante dos eventos das guardas (limite sugerido: mais de 20% das leituras guardadas) **e** os tokens evitados estimados pelas guardas são pequenos frente ao custo real de input (sugerido: abaixo de 5%), o que indicaria que as guardas bloqueiam sem repor o conteúdo; e a taxa de acerto L3 do cache mostra acesso repetido aos mesmos artefatos.
- **Critérios para seguir com a F7:** uma fatia medida de prompts idênticos byte a byte acima de 10% (nenhuma é esperada: prompts agênticos quase não se repetem, hooks não interceptam chamadas à API e a Anthropic já faz cache do prefixo no servidor). Sem essa evidência o proxy continua fora do escopo, como dizem os não-objetivos da spec.
- **Consequências:** nada se perde: o modelo de dados e os eventos dos hooks já registram o que os critérios precisam. Rever esta entrada quando os critérios puderem ser avaliados.


## 2026-10-28 — Hooks: onde a documentação oficial vence a spec

Confirmado em <https://code.claude.com/docs/en/hooks> ao implementar as fases F3 a F5:

- O modo `warn` emite só `additionalContext`. Nunca emite `permissionDecision: "allow"`, porque isso pularia o prompt de permissão do próprio usuário.
- O `Stop` emite só `systemMessage` (uma mensagem para o usuário). `additionalContext` ou `decision` no `Stop` fariam o Claude continuar, o que a spec proíbe. O `SessionStart` não imprime nada, porque o stdout dele vira contexto do modelo.
- O `source` do `SessionStart` tem um quinto valor, `fork`; como `startup` e `resume`, não é um corte (só `clear` e `compact` são).
- As leituras de arquivo são rastreadas por `sessão/agent_id`, porque o contexto de um subagente é separado do pai. Uma leitura no mesmo segundo do corte é permitida (`>` estrito).
- Os globs de bloqueio casam o path relativo ao `cwd` da sessão. Imagens e PDFs ficam isentos da checagem de tamanho. Um `Read` com qualquer argumento além de `file_path`, `offset` e `limit` (como `pages`) pula a checagem de releitura. Os bytes evitados por uma leitura em faixa são estimados em 80 bytes por linha, e arquivos acima de 16 MiB não são hasheados.
- O `guard-search` conta arquivos por raiz de repositório (em cache na camada de cache, chave = raiz, limite e mtime da raiz), para de contar em limite + 1 e desiste após 100 ms, caso em que permite a busca.
- O `route` grava só classes acionáveis em `classifications` (nunca `other`), faz cache das classificações por `blake3(prompt)` mais uma versão das regras, e no modo `enforce` se comporta como `warn`. No `enforce`, architect e planner são intercambiáveis.
- O `session-check` avisa uma vez por sessão, de novo depois que o contexto cresce metade do limite, ou após um clear/compact. Um `~/` literal no `transcript_path` não é expandido.
- O stdout não pode ser fechado (`unsafe` é proibido): a resposta é descarregada e a linha do banco é gravada por último, mas o Claude Code só vê o fim quando o processo termina. Um hook nunca cria a pasta de dados nem o `tokred.db`; um `tokred.toml` inválido significa os padrões. Um watchdog (`TOKRED_HOOK_GUARD_MS`, padrão 2 s) transforma um hook travado em um resultado `timeout_guard`.
- Medido num Mac de desenvolvimento ocioso, com build de release: p95 de spawn até o fim de 10 a 12 ms (cerca de 7 ms disso é a base do spawn), p95 interno de até cerca de 6 ms. Sob carga o p95 chegou a 37 ms. Tudo dentro do orçamento de 50 ms.

## 2026-10-28 — Preços, update e o caminho do self-update

- O snapshot de preços mantém o formato do próprio LiteLLM com chaves normalizadas, então um único parser lê tanto a lista online quanto o arquivo embutido (39 linhas, regeneradas por `scripts/update-price-snapshot.sh`). Preferência de chaves: chaves Anthropic sem prefixo, depois `anthropic/`, `vertex_ai/`, `bedrock/`, depois o nativo do Bedrock `anthropic.`; chaves regionais e de revendedores são descartadas. Preços de cache ausentes são derivados (escrita 1,25x, leitura 0,1x do input).
- "Preço mais recente de uma família" é a entrada com a data mais recente no id do modelo; ids sem data ficam abaixo dos datados.
- Um sync que não devolve linhas nunca substitui a tabela. O `price_sync_attempt_at` é gravado para que um sync que falhou não seja repetido a cada comando.
- O `BinarySwap` é um port do `tokred-update` (o port `FileSystem` não tem rename nem chmod). O self-update verifica o SHA-256 antes de tocar em qualquer coisa, troca o binário de forma atômica e, no Windows, renomeia o `.exe` antigo para `.old` e o remove na execução seguinte. Linux sempre mapeia para o build musl. As tags de release são `v<versão>`.
- Depois de um self-update a configuração é reaplicada rodando o binário **novo** (`configure --yes`), para que os templates de agentes instalados sejam os novos. Em instalações por gerenciador de pacotes, o primeiro comando da CLI depois de um upgrade reaplica a configuração em silêncio (`needs_reconfigure`).
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL` e `TOKRED_RELEASES_URL` são acréscimos à spec: um único interruptor para toda chamada de rede, e substituição de URLs para espelhos e para testes contra um servidor local.

## 2026-10-28 — Cálculos das métricas

- O contrafactual "tudo no opus" usa o preço **mais recente** de opus da tabela. Quando o modelo realmente usado é um opus mais antigo e mais caro, o contrafactual pode ficar abaixo do custo real; a economia por roteamento fica então negativa, o que é correto.
- Um "bloqueio" de guarda é um evento `guard-*` com decisão `deny`; `warn` não economiza nada. A economia das guardas é sempre rotulada estimativa. O modelo da sessão usado para precificá-la é o de mais mensagens (empate: menor id; fallback opus).
- O divisor da projeção é `min(janela, dias desde a primeira linha)`, no mínimo 1. `TimeRange::last` é `[agora - período, agora + 1)`.
- Um arquivo substituído por outro maior não é detectado pela ingestão, porque não há hash do conteúdo; um arquivo que encolheu recomeça do zero.
- As classificações do `route` são contadas a partir dos eventos do hook `route`, então não há tabela de classificações separada para o relatório consultar.

## 2026-10-28 — Audit: locais confirmados na documentação oficial

- Servidores MCP não são configurados no `settings.json`. O escopo de usuário é o `mcpServers` de nível superior do `~/.claude.json`, o escopo local é `projects["<path>"].mcpServers` no mesmo arquivo, o escopo de projeto é `<projeto>/.mcp.json`; a precedência é local, depois projeto, depois usuário. O `settings.json` só contribui com `disabledMcpjsonServers`. Não é documentado se o `.claude.json` acompanha o `CLAUDE_CONFIG_DIR`, então `<claude>/.claude.json` é tentado primeiro e depois `~/.claude.json`.
- A memória do projeto também inclui `.claude/CLAUDE.md`, `CLAUDE.local.md`, diretórios ancestrais e os arquivos `.claude/rules` sempre ativos. Os imports seguem o limite documentado de 4 níveis e ignoram blocos de código, trechos inline e comentários HTML de bloco.
- A sugestão "dividir com @import" da spec foi trocada por "mover para rules com escopo de path ou uma skill", porque imports não reduzem o custo de contexto. Corpos de skills só carregam quando invocados, então as skills também informam o peso na inicialização. Limites: 200 linhas para `CLAUDE.md` e 500 para skills (ambos da documentação) e 2000 tokens para agentes (uma heurística do tokred).
- Skills, agentes e servidores de plugins, `commands/*.md`, conectores do claude.ai e servidores gerenciados não são enumerados; o uso visto nos transcripts de servidores não configurados localmente é listado à parte.

## 2026-10-28 — Pipeline de release e licenças

- O `dist` constrói seis alvos (macOS, Linux musl e Windows em x86_64 e aarch64) com arquivos `.tar.gz` / `.zip` e sidecars `.sha256`. Os templates dos agentes saíram de `templates/agents/` para `crates/tokred-install/templates/` porque o `include_str!` a partir da raiz do repositório quebraria o crate publicado.
- O `dist` não consegue adicionar caveats do Homebrew, então o `publish-package-managers.yml` espera a fórmula da nova versão chegar ao tap e inclui o aviso nela. O manifest do Scoop, o pull request do winget e o `cargo publish --workspace` também estão nesse workflow. Nada disso rodou ainda: precisa dos quatro secrets do repositório e de uma tag, e o pacote do winget precisa ter o primeiro manifest enviado à mão.
- O `deny.toml` permite `CDLA-Permissive-2.0` (os dados de certificados da Mozilla embutidos pelo `webpki-roots`, trazido pelo cliente HTTP) além da exceção de `MPL-2.0` para o `option-ext`.

## 2026-10-28 — Painéis no terminal, tarefas de fundo e o benchmark

- `tokred stats`, `stats latency` e `metrics` imprimem um painel colorido no terminal (o relatório HTML são os mesmos dados, não um substituto). A cor segue `NO_COLOR` (sempre vence), `CLICOLOR_FORCE`, um `TERM` dumb e se o stdout é um terminal.
- Os avisos das tarefas de fundo vão para o **stderr**, para que a saída `--json` continue limpa. As tarefas rodam só depois de comandos que não são hooks, são de melhor esforço e nunca fazem o comando falhar: a sincronização diária de preços, a verificação diária de versão e a reaplicação silenciosa da configuração depois de um upgrade.
- O `doctor --bench` executa cada hook como um processo separado numa pasta de dados temporária com banco próprio (os hooks fazem trabalho real e os dados do usuário ficam intactos), cronometra do spawn até o fim e guarda a diferença para as medições internas dos hooks como o overhead de inicialização do processo, em `meta`. `TOKRED_BENCH_RUNS` substitui o padrão de 50 execuções por payload (usado nos testes).
- A cobertura de linhas é 100% no workspace inteiro. O resumo do LLVM pode mostrar uma fração de ponto percentual a menos porque conta regiões de macro e ramos de erro do `?` à parte; as visões por linha (`lcov`, texto anotado) são a referência.


## 2026-10-28 — Spec v3: o proxy e uma política de modelo que respeita a escolha do usuário

- **Contexto:** o rascunho v2 propunha um proxy local com compressão de resultados de ferramentas, cache de respostas por correspondência exata e medição, mas sem roteamento de modelo. A leitura do guia oficial de gateways, da página de cache de prompt e da de preserved thinking mostrou que (a) o Claude Code rotula cada requisição (`x-claude-code-request-class`, tipo e id do agente, id do prompt, id da sessão), (b) gateways devem inspecionar sem modificar e reescrever o histórico pode causar o erro "bound to a different conversation", (c) trocar o modelo principal no meio da sessão reconstrói todo o cache de prompt e (d) requisições auxiliares podem rodar no modelo principal e algumas reaproveitam o cache da conversa.
- **Decisão (combinada com o autor):** o proxy classifica as requisições e pode escolher o modelo **apenas** de subagentes e requisições auxiliares; o **loop principal mantém o modelo configurado pelo usuário** e o tokred só **resume e sugere** um modelo mais barato quando a tarefa foi simples, de preferência quando o cache já está frio. Compressão e cache de respostas ficam **só em modo sombra** até um portão de dados passar. Todo comportamento que altera tráfego começa em modo `observe`.
- **Consequências:** o `docs/spec.pt-BR.md` (v3) substitui a v1 e o rascunho v2, que ficam em `docs/archive/`. As antigas fases F6/F7/F9 são substituídas por F6 (estabilização), F7 (spike), F8 (repasse e medição), F9 (política de modelo), F10 (sugestões e sombra) e F11 (compressão ativa opcional). A falha do CI no Windows, as famílias de modelo que faltam (Fable, Mythos) e os multiplicadores de cache por modelo ficam registrados como problemas conhecidos a corrigir primeiro.


## 2026-10-28 — Spike F7, primeiros resultados (login por chave de API; assinatura ainda não testada)

Feito com um proxy de repasse descartável (`scripts/spike/`) e o Claude Code 2.1.312 real em modo `-p`, usando o login por chave de API desta máquina (uma chave gerenciada pela organização, `/login managed key`).

- **O repasse funciona.** O streaming (SSE) é repassado sem alteração e o `usage` é legível em `message_start` / `message_delta`. O `model` da resposta bate com o da requisição.
- **Cabeçalhos realmente enviados:** `x-claude-code-session-id` em toda requisição e `x-claude-code-agent-id` só nas de subagentes. `x-claude-code-request-class`, `x-claude-code-agent-type` e `x-claude-code-prompt-id`, que o guia de gateways diz que são enviados, **não** apareceram nesta versão e modo. Consequência: a classificação não pode depender deles. Principal versus subagente se distingue pela presença do `agent-id`; o tipo de subagente, pelo conjunto de ferramentas (o subagente Explore tinha 10 ferramentas só de leitura contra 15 do loop principal) e pelos blocos de system. Os cabeçalhos documentados são um bônus quando existirem, a reconferir no modo interativo.
- **Subagentes montam o próprio cache** (a primeira volta gravou cerca de 19,9 mil tokens e as seguintes os leram de volta), por isso o modelo de um subagente deve ser decidido uma vez e mantido.
- **Reescrever o modelo das requisições de subagente funciona.** Com o loop principal no Sonnet, reescrever só as requisições do subagente para Haiku foi aceito pela API nas três voltas (inclusive com resultados de ferramentas voltando), o Claude Code não mostrou problema com a resposta de outro modelo e a resposta final estava correta. As requisições do loop principal nunca foram tocadas.
- **Ainda não verificado:** login por **assinatura** claude.ai (esta máquina não tem: o `authMethod` é `api_key`); modo interativo (os outros cabeçalhos podem aparecer lá); requisições `auxiliary` (nenhuma ocorreu no modo `-p`); compactação; modelos com muito thinking em uma reescrita. A sonda e os passos exatos para testar assinatura sem mexer num login real estão em `scripts/spike/README.pt-BR.md`.
- **Decisão:** o modelo do loop principal **nunca** é trocado, sem nenhuma opção de configuração para isso; o tokred apenas aconselha.


## 2026-10-28 — Spike F7, resultado da assinatura (login claude.ai): funciona

Mesma sonda descartável e Claude Code 2.1.312 da rodada com chave de API, agora com login por **assinatura** claude.ai (`authMethod: claude.ai`, `ANTHROPIC_API_KEY` removida), só com `ANTHROPIC_BASE_URL`.

- **O repasse funciona com assinatura.** Status 200 com streaming. A credencial viaja no cabeçalho `authorization` (OAuth), não em `x-api-key`, e o valor `oauth-2025-04-20` do `anthropic-beta` está presente, então o proxy deve repassar o `authorization` e o `anthropic-beta` inteiro sem tocar, exatamente como diz o guia de gateways. A requisição de assinatura também levou `extended-cache-ttl-2025-04-11` (o cache de 1 hora).
- **Os cabeçalhos são os mesmos da chave de API:** `x-claude-code-session-id` em toda requisição e `x-claude-code-agent-id` só nas de subagentes; sem `request-class`, `agent-type` nem `prompt-id` no modo `-p`.
- **Reescrever o modelo de um subagente funciona com assinatura.** Com o loop principal no Sonnet, reescrever só as três requisições do subagente para Haiku foi aceito (status 200 em todas) e a resposta final estava correta. As duas requisições do loop principal nunca foram tocadas.
- **Não coberto, portanto ainda em aberto:** os termos de uso da Anthropic para tráfego de assinatura por proxy (não se verifica testando; precisa ser lido e registrado antes de recomendar o proxy para assinaturas); limites do plano por modelo (este teste só moveu Sonnet para Haiku, um modelo mais barato); modo interativo, requisições `auxiliary` e compactação; sessões muito longas.
- **Consequência:** o go/no-go da F7 é **go** para os dois tipos de login do ponto de vista técnico. O proxy continua opt-in, e a recomendação para assinaturas depende da verificação dos termos de uso.


## 2026-10-28 — Spike F7, modo interativo: como reconhecer requisições auxiliares

`claude` interativo (2.1.312, Opus 5.5, login por chave de API) passando pela sonda, dois prompts curtos.

- **O `x-claude-code-request-class` também não é enviado no modo interativo.** Só chegou o `x-claude-code-session-id` (e o `x-claude-code-agent-id` nos subagentes, nas rodadas anteriores). Não dá para depender dos cabeçalhos documentados de classe, tipo de agente e id de prompt nesta versão.
- **A requisição auxiliar é reconhecível pelo formato.** Além das voltas do loop principal (16 ferramentas, thinking adaptativo), uma requisição tinha **nenhuma ferramenta, nenhum thinking e exatamente uma mensagem** (1.167 tokens de entrada e 21 de saída, muito provavelmente o título da sessão). Ela rodou no modelo principal (Opus 5.5) e **não** leu nem gravou cache de prompt, então movê-la para um modelo mais barato não perde nada.
- **Verificações de inicialização:** doze requisições com `max_tokens: 1`, sem ferramentas e sem system prompt devolveram `429` e foram repetidas pelo Claude Code; o proxy precisa repassar (e repassou) o `429` sem alteração.
- **Decisão:** uma requisição só é tratada como auxiliar quando não tem `agent-id`, não tem ferramentas, não tem `thinking` e tem exatamente uma mensagem (ou quando um Claude Code futuro enviar `x-claude-code-request-class: auxiliary`). O loop principal sempre leva ferramentas, então nunca se encaixa. Todo o resto sem `agent-id` é loop principal e nunca é reescrito.


## 2026-10-28 — Estabilização F6

- **Gravações de cache de 1 hora.** Os transcripts atuais dividem o `cache_creation_input_tokens` em `cache_creation.ephemeral_5m_input_tokens` e `ephemeral_1h_input_tokens`, e nesta máquina todas as gravações eram de 1 hora. O LiteLLM publica o preço como `cache_creation_input_token_cost_above_1hr`. `Usage` ganhou `cache_write_1h` (uma parte de `cache_write`) e `ModelPrice` ganhou `cache_write_1h`; quando a lista de preços não o tem, vale a tarifa documentada de 2x o input. A migration v3 adiciona as colunas e limpa os offsets de ingestão para que as linhas existentes sejam corrigidas sem duplicar.
- **Ordem dentro da família.** A versão agora vem antes da data (`claude-opus-5-5` > `claude-opus-4-7-20260416`). A regra anterior, que punha ids sem data abaixo dos datados, fazia o contrafactual precificar tudo como Opus 4.7.
- **Fable e Mythos** são famílias. O tipo `Family` continua um enum fechado (famílias desconhecidas continuam saindo como "sem preço") em vez do tipo aberto que a spec sugeria: assim o `match` continua exaustivo, e uma família desconhecida já é tratada.
- **Windows.** Os metadados em cache da varredura foram trocados por uma leitura nova com `std::fs::metadata`, e o CI roda `cargo test --no-fail-fast` para que todo binário de teste com falha apareça, não só o primeiro.
- **Não feito:** exercitar o pipeline de release com uma tag de teste. Enviar uma tag é uma ação externa (pode publicar no Homebrew, Scoop, winget e crates.io quando os secrets existirem) e precisa do aval explícito do autor.


## 2026-10-28 — Tráfego de assinatura pelo proxy: o que dizem os termos

Conferido nos Termos do Consumidor da Anthropic (seção 3, item 7) e na documentação de gateways do Claude Code. É uma leitura do texto publicado, não um parecer jurídico.

- Os Termos do Consumidor proíbem acessar os Serviços "through automated or non-human means", exceto com chave de API "or where we otherwise explicitly permit it". Não citam proxies locais nem gateways.
- A documentação do Claude Code descreve explicitamente a configuração que o tokred usa: só com `ANTHROPIC_BASE_URL`, "a saved claude.ai login remains the active credential" e os gateways "must forward the OAuth capability in `anthropic-beta`". **Medir** o tráfego de assinatura pelo proxy local é, portanto, uma configuração documentada.
- **Reescrever o modelo** de requisições de assinatura não é tratado em nenhum dos dois textos. Reduz o uso em vez de contornar limites, mas não é permitido explicitamente. Decisão: o roteamento fica em `observe` por padrão em qualquer login, e o `doctor` avisa quando o `enforce` estiver ligado com login de assinatura, para que ligá-lo seja uma escolha informada do usuário.


## 2026-10-28 — O proxy pede `Accept-Encoding: identity` ao upstream

- **Contexto:** a API real comprime (gzip) as respostas quando o cliente permite, e o proxy precisa ler o `usage` do corpo da resposta para medir os tokens com exatidão.
- **Decisão:** o proxy reescreve o `Accept-Encoding` para `identity` na requisição que envia ao upstream. É o único cabeçalho de requisição que ele reescreve; todos os outros seguem como vieram.
- **Consequências:** as respostas chegam sem compressão (alguns bytes a mais entre o proxy e a API), então o streaming e o `usage` podem ser lidos enquanto passam. Se um upstream comprimir mesmo assim, a resposta é encaminhada sem alteração e o uso dela não é lido (fail-open, veja os riscos conhecidos abaixo).

## 2026-10-28 — Como o proxy em segundo plano é ligado

- **Contexto:** o `tokred proxy start` precisa deixar um proxy rodando depois que o comando retorna, e o `proxy stop` e o `tokred claude` precisam encontrá-lo e falar com ele com segurança.
- **Decisão:** o proxy em segundo plano é o mesmo binário, iniciado como `tokred proxy start --foreground` (o serviço de usuário executa o mesmo comando), então não há um segundo executável para instalar nem manter em sincronia. O estado fica em `<dados>/proxy/`: `secret` (só do dono, `0600` em Unix), exigido pelos endpoints de administração (`/__tokred/...`), e `pid`. O `proxy stop` envia o pedido de encerramento e **espera até o processo sumir** (falha se o proxy não parou) antes de apagar o arquivo de pid.
- **Consequências:** `proxy stop && proxy start` é seguro em scripts (o `stop` só retorna depois que o proxy parou de responder); um arquivo de pid velho é ignorado quando o proxy não responde ao endpoint de saúde.

## 2026-10-28 — O circuit breaker da compressão sobrevive a uma reinicialização

- **Contexto:** quando requisições reescritas continuam sendo rejeitadas, o breaker desliga a compressão. Só em memória, uma reinicialização a ligaria de novo e as mesmas falhas se repetiriam.
- **Decisão:** o disparo é gravado na tabela `meta` do banco (`compression_tripped_at`). Enquanto ele existir, o proxy inicia com a compressão em `observe` mesmo que o `tokred.toml` diga `enforce`; o `tokred compression on` o apaga.
- **Consequências:** uma reinicialização (ou um boot com o serviço de usuário) nunca religa em silêncio um recurso que estava falhando; o `tokred compression status` mostra que o breaker disparou.

## 2026-10-28 — `tokred compression on|off` só edita a configuração do Claude Code quando executado

- **Contexto:** a compressão reversível precisa do servidor MCP `tokred_retrieve` registrado no Claude Code, o que significa editar a configuração de usuário dele (`~/.claude.json`), um arquivo que guarda mais do que dados do tokred.
- **Decisão:** só o `tokred compression on` (registra o servidor e define `[compression] mode = "enforce"`) e o `off` (remove o registro e define `observe`) tocam nesse arquivo, e só quando você os executa. O conteúdo anterior é salvo antes ao lado dele como `.tokred.bak`. O `configure`, o `tokred claude` e o proxy nunca o editam.
- **Consequências:** instalar ou usar o proxy não muda nada na configuração do Claude Code; ligar a compressão é um passo explícito e reversível.

## 2026-10-28 — Riscos conhecidos do proxy

- **Compactação parece um prompt simples.** Requisições de compactação não trazem cabeçalho que as distinga na versão do Claude Code testada, então uma delas pode ser lida como prompt simples e alimentar uma sugestão de modelo mais barato. As sugestões só aconselham (o modelo do loop principal nunca é alterado) e `routing.compaction` fica em `keep` por padrão, então o custo de uma leitura errada é uma sugestão errada.
- **Uso não lido quando o upstream comprime mesmo assim.** Se uma resposta chegar comprimida apesar do `Accept-Encoding: identity`, ela é encaminhada sem alteração e o uso daquela requisição simplesmente não é registrado (fail-open: o Claude Code nunca é afetado, a medição fica com uma lacuna).
- **O roteamento fica em `observe` por padrão** em qualquer login, inclusive de assinatura, porque os termos da Anthropic não tratam da reescrita do modelo de requisições de assinatura (veja a decisão sobre tráfego de assinatura acima). Ligar o `enforce` é uma escolha informada do usuário, e o `doctor` avisa quando ele está ligado com login de assinatura.

## 2026-10-28 — Windows: um `proxy start` capturado espera o proxy

- **Contexto:** a primeira execução do CI dos testes de ponta a ponta do proxy travou só no Windows. O harness de testes captura a saída do `tokred proxy start` por pipes; o proxy destacado herda esses handles de pipe (o Windows repassa ao filho todos os handles herdáveis do pai, sejam quais forem os handles padrão do próprio filho), então o harness só vê o fim da saída quando o proxy termina.
- **Decisão:** os testes iniciam o proxy sem capturar a saída, e o limite está documentado em [proxy](proxy.pt-BR.md). Fechar os handles herdados exige `unsafe` ou um crate de plataforma, o que o workspace hoje não permite; uma alternativa a avaliar depois é iniciar o processo pelo shell (`Start-Process`), que não herda handles. Os jobs de CI agora param após 20 minutos, para que um travamento falhe com o seu log.
- **Consequências:** um terminal, um redirecionamento para arquivo, o `NUL` ou o serviço de usuário funcionam; capturar a saída do `proxy start` num pipe no Windows bloqueia até o proxy parar.

## 2026-10-29 — O painel: compromisso de privacidade, deltas por turno, login e encerramento

- **Contexto:** o proxy agora serve um painel ao vivo (`/dashboard`, `/metrics`) que mostra o que foi enviado ao Claude e o que voltou. A promessa anterior de que o conteúdo das mensagens nunca é guardado não se sustenta se isso for exibido, e o Claude Code reenvia a conversa inteira a cada requisição.
- **Decisão (privacidade):** o conteúdo só é capturado com `[proxy] log_bodies = true`, desligado por padrão. Desligado significa que nada do conteúdo é lido das requisições ou respostas, mantido em memória, enviado ao navegador ou guardado (o evento `turn_delta` também é conteúdo e não é enviado); o painel continua mostrando metadados (modelos, tokens, custo, tempos, status, motivo de parada, nomes das ferramentas, roteamento, subagentes). Ligado significa **deltas** por turno, não corpos: as mensagens novas desde a requisição anterior da mesma sessão e subagente (acompanhadas por um hash por mensagem, com uma checagem rápida da primeira e da última; após uma compactação ou retrocesso são enviadas as mensagens que diferem) e a resposta remontada, cada texto truncado em 20.000 caracteres. Cabeçalhos e credenciais nunca são capturados. O conteúdo fica em `captured_turns` (migration v5) e é limpo ao iniciar e a cada hora: só as sessões ativas ou entre as últimas 10 o mantêm, e nada passa de `retention_days`. O motivo de parada e os nomes das ferramentas são metadados, então `captured_turns` tem uma linha por turno e só as colunas de conteúdo dependem de `log_bodies`; a limpeza esvazia o conteúdo e mantém a linha até `retention_days`. A documentação que dizia que o conteúdo nunca é guardado agora diz isto, de forma condicional.
- **Decisão (login):** outros usuários locais alcançam o loopback e um navegador não consegue enviar o `x-tokred-secret`, então `POST /__tokred/dashboard-token` (com o segredo) emite um token válido por 60 segundos e uma vez; `GET /dashboard?token=` o troca por um cookie aleatório (`HttpOnly`, `SameSite=Strict`, `Path=/` para o `/metrics` compartilhá-lo) e redireciona. Tokens e cookies são comparados em tempo constante contra todos os valores guardados e ficam na memória (reiniciar encerra o login). A aleatoriedade vem do `RandomState`, como o segredo de administração, para evitar uma dependência nova. Requisições com `Sec-Fetch-Site: cross-site` ou `same-site` são recusadas (`same-site` também: outro app web local em outra porta é o mesmo site), nenhum cabeçalho CORS é enviado e toda página leva uma CSP estrita. O `/metrics` tem a própria CSP (`style-src 'unsafe-inline'`) e permite exatamente os scripts embutidos do relatório pelo hash SHA-256, então o seletor de período e as dicas continuam funcionando enquanto nada mais pode executar.
- **Decisão (eventos e mesclagem):** um canal broadcast limitado de eventos já formatados; nada é montado quando ninguém escuta, um assinante lento perde os eventos mais antigos (ele relê a API ao reconectar; deltas nunca são reenviados). O proxy mantém em memória os turnos em andamento e os últimos encerrados e os mescla por id de requisição sobre o que o banco devolve, a versão ao vivo vencendo, então uma sessão mostra os dois. Os subagentes em execução são acompanhados pelo id de agente das requisições em andamento (em execução enquanto uma está em andamento e por 60 segundos após a última); uma tarefa de varredura os encerra e termina com o proxy.
- **Decisão (encerramento):** o hyper espera um corpo em streaming em andamento quando se pede que uma conexão pare, então um stream de eventos aberto seguraria o proxy até o tempo de drenagem (10 s, mais que o `proxy stop` espera). A tarefa do stream de eventos faz `select` no sinal de encerramento, então os streams abertos terminam na hora; um teste de ponta a ponta abre `/dashboard/events`, roda `proxy stop` e verifica que o stream termina e o processo sai.
- **Consequências:** ligar `log_bodies` guarda texto de conversas no banco local; os dados nunca saem da máquina e são limpos, mas quem puder ler o banco pode lê-los. Uma requisição de compactação aparece como `auxiliary` no painel. Só os 500 turnos mais recentes de uma sessão são devolvidos (`truncated: true`). Requisições sem id de sessão não são acompanhadas. O relatório do `/metrics` é gerado dos transcripts a cada requisição, numa tarefa bloqueante, sem a auditoria do projeto.
