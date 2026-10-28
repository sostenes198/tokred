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
