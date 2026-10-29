[English](spec.md) | **Português**

# tokred — Especificação (v3, final)

**Repositório:** https://github.com/sostenes198/tokred

**Descrição curta (GitHub "About"):** *Token and cost reducer for Claude Code — local proxy with model policy, context guards and metrics. Works alongside rtk.*

**Descrição:** tokred é uma CLI em Rust, local e multiplataforma, que reduz o custo e o consumo de tokens do Claude Code. Ela tem duas peças que se complementam: **hooks** que impedem conteúdo inútil de entrar no contexto, e um **proxy local** que fica entre o Claude Code e a API da Anthropic para medir cada requisição com exatidão, escolher o modelo das tarefas auxiliares e dos subagentes, e sugerir ao usuário quando um modelo mais barato bastaria. Tudo roda na máquina do usuário, com um SQLite próprio, sem serviço externo. Roda junto com o rtk (rust token killer), que comprime a saída de comandos.

> **Esta é a versão final da spec.** Ela substitui a v1 e o rascunho v2 (ambos em [`docs/archive/`](archive/)). Ela diz **o que já foi feito**, **o que muda** e **o que falta** para o projeto funcionar como proxy. As partes já implementadas são definidas pelos documentos de `docs/` (seção 2); aqui elas aparecem resumidas.

---

## 0. Instruções para o Claude Code

- Antes de implementar **cada hook**, consulte a documentação oficial de hooks (https://code.claude.com/docs/en/hooks). Antes de implementar **qualquer parte do proxy**, consulte também o guia de compatibilidade de gateways (https://code.claude.com/docs/en/llm-gateway-protocol.md), a página de cache de prompt e a de *preserved thinking* da API. **Não assuma nomes de campo nem de cabeçalho.** Onde esta spec diverge da documentação, a documentação vence: registre a divergência em `docs/decisions.md`.
- Implemente por fases (seção 12). Ao final de cada fase: testes passando nos 3 SOs, `cargo clippy -- -D warnings` limpo, cobertura de linhas mantida e um resumo do que foi feito.
- Pergunte antes de qualquer decisão que mude o comportamento descrito aqui.
- **Nunca faça mais de um `git push` por pedido do usuário** e nunca sem autorização explícita.

### Regras de idioma

- **Código em inglês:** pastas, arquivos, crates, módulos, tipos, funções, variáveis, tabelas e colunas, chaves da config, comentários, mensagens de commit e todas as strings exibidas pela CLI, pelos hooks e pelo proxy.
- **Documentação sempre nos dois idiomas** (`X.md` + `X.pt-BR.md`, com o mesmo conteúdo), atualizados no mesmo commit. Documentos em `docs/archive/` são exceção (histórico).
- As palavras-chave do classificador continuam em pt-BR e en (são dados).

---

## 1. Visão e o que mudou desde a v1

### 1.1 O problema

O custo de uma sessão do Claude Code é dominado por **contexto reenviado a cada turno** (lido do cache a 0,1× do preço de input, ou reescrito a 1,25× quando o cache falha) e pelo **modelo** usado em cada requisição. Hooks conseguem impedir que conteúdo inútil entre no contexto, mas **não enxergam a requisição ao modelo**: não sabem exatamente o que foi cobrado, não sabem por que o cache falhou e não decidem qual modelo atende cada chamada.

### 1.2 A ideia nova

Um **proxy local** (`tokred proxy`) vê cada requisição e cada resposta. Isso dá três coisas que os hooks não dão:

1. **Medição exata** (uso por requisição, por tipo de requisição, por subagente, por prompt) e **explicação de cache miss**.
2. **Política de modelo** para o que o usuário não escolheu diretamente (subagentes herdados, tarefas auxiliares).
3. **Sugestão fundamentada** de modelo mais barato para o loop principal, sem nunca trocá-lo por conta própria.

### 1.3 O que a documentação oficial estabelece (verificado)

Estes fatos moldam a spec inteira; foram lidos nas páginas oficiais da Claude Code e da API:

1. **O Claude Code identifica cada requisição para gateways.** Cabeçalhos `x-claude-code-request-class` (`main`, `subagent`, `workflow`, `compaction`, `auxiliary`), `x-claude-code-session-id`, `x-claude-code-agent-id`, `x-claude-code-parent-agent-id`, `x-claude-code-agent-type` (`Explore`, `Plan`, `general-purpose`, `custom`, `teammate`, `fork`), `x-claude-code-prompt-id`, `x-claude-code-compaction`, `x-claude-code-context-compacted` e `x-claude-code-prev-tool-durations`. O proxy classifica o tráfego **sem interpretar o corpo**. Os cabeçalhos são uma lista aberta: tratar como tal. **Verificado no spike F7** (Claude Code 2.1.312, modo `-p`, chave de API): só chegaram `x-claude-code-session-id` (todas as requisições) e `x-claude-code-agent-id` (só subagentes); `request-class`, `agent-type` e `prompt-id` **não** apareceram. Portanto a classificação usa a presença de `agent-id`, o conjunto de ferramentas e os blocos de system, e trata os demais cabeçalhos como bônus quando existirem (conferido também no modo interativo: o `request-class` não chega). **Requisição auxiliar** é reconhecida pelo formato: sem `agent-id`, sem ferramentas, sem `thinking` e com exatamente uma mensagem (ex.: o título da sessão); o loop principal sempre tem ferramentas e nunca se encaixa.
2. **Gateways devem "inspecionar sem modificar".** Campos do corpo e cabeçalhos `anthropic-beta` vêm em pares; reescrever o corpo quebra o par e gera `400`. O corpo e os `anthropic-*` devem ser repassados sem filtragem por lista fixa.
3. **Preserved thinking.** A API rejeita (`400`, "bound to a different conversation") quando `system`, `tools` ou `messages` anteriores diferem do que existia quando o bloco de thinking foi produzido. Um gateway que reescreve esse conteúdo pode causar a rejeição. Trocar o `model` entre turnos **não** viola essa checagem, mas blocos que o novo modelo não consegue ler são descartados sem erro.
4. **Cache de prompt.** O cache é por prefixo idêntico (ferramentas → system → messages) e quebra quando algo anterior muda. TTL padrão de 5 minutos (1 hora opcional); escrita custa 1,25× (5 min) ou 2× (1 h) o preço de input; leitura custa 0,1× (0,05× no Opus 5.5 e no Sonnet 5.5, 0,025× no Fable 5.1 e no Mythos 5.1). Mínimos cacheáveis variam de 512 a 4.096 tokens por modelo. Trocar de modelo no loop principal reescreve o contexto inteiro.
5. **Assinatura.** Definir só `ANTHROPIC_BASE_URL` mantém o login claude.ai ativo; o gateway precisa repassar o valor OAuth de `anthropic-beta`. A Anthropic não suporta rotear o Claude Code para modelos que não sejam Claude.
6. **Requisições auxiliares** (títulos, classificadores, resumos) usam, conforme o tipo de login, o modelo principal ou o Haiku. Algumas (sugestão de próximo prompt) **reaproveitam o cache da conversa**.
7. **O Claude Code já oferece controles de modelo sem proxy:** `ANTHROPIC_DEFAULT_*_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL`, `model:` no frontmatter do subagente, `opusplan`, níveis de esforço. O proxy acrescenta aplicação por requisição, medição completa (inclusive subagentes) e visibilidade; **não** é a única forma de rotear.
8. **Modelos novos aparecem com frequência** (famílias Fable e Mythos, multiplicadores de cache diferentes por modelo). Nada sobre modelos pode ficar fixo em código: tem de vir de dados versionados e atualizáveis.

### 1.4 Mudanças de decisão em relação à v2

| v2 | v3 |
|---|---|
| Proxy faz compressão de `tool_result` ativa | Só em **modo sombra** (calcula o que economizaria, não altera nada). Ligar de verdade exige o portão da fase F10 (seção 12) |
| Proxy sem roteamento de modelo | **Política de modelo** para subagentes e tarefas auxiliares; loop principal **intocado** |
| `route` hook como classificador principal | Os cabeçalhos do Claude Code substituem o classificador por palavras-chave **para medir**; o `route` vira sugestão opcional |
| Cache de respostas (fase F8) | Só **sombra** (mede repetição por hash); cache ativo fica no backlog |
| Cache de artefatos (F9) | Backlog |
| Sem compressão reversível | **Compressão reversível** de `tool_result` grandes (fase F11): o proxy troca o bloco por prévia + id de forma determinística e o modelo recupera o original pela ferramenta `tokred_retrieve` do servidor MCP `tokred mcp` (seção 6.7). Começa em `observe` |

### 1.5 Não-objetivos

- Comprimir saída de Bash no hook (papel do rtk) e depender do `rtk-proxy`.
- Trocar o modelo do **loop principal** (apenas sugerir).
- Rotear para modelos não-Claude.
- Cache semântico, Redis ou qualquer serviço externo.
- Modificar `system`, `tools`, `thinking`, `cache_control`, imagens ou blocos de thinking.

---

## 2. Estado atual: o que já foi feito

Tudo abaixo está em `main` (repositório publicado). Os comportamentos exatos estão nos documentos indicados e nos registros de `docs/decisions.md`.

### 2.1 Entregue (fases F0 a F5 da v1)

| Componente | Estado | Onde está definido |
|---|---|---|
| Workspace de 10 crates, CI, `cargo-deny`, MSRV 1.89, docs bilíngues | Feito | `docs/development.md`, `docs/architecture.md` |
| `configure` / `unconfigure` / `doctor` com merge não destrutivo, agentes `tokred-*` e manifesto | Feito | `docs/commands.md`, `docs/agents.md` |
| Seis hooks fail-open: `guard-read`, `guard-search`, `session-start`, `route`, `enforce`, `session-check`, com registro de latência | Feito | `docs/hooks.md` |
| Store SQLite (schema v2, migrations), cache em camadas L1/L2/L3 | Feito | `docs/cache.md` |
| Preços: lista LiteLLM, snapshot embutido, sync diário | Feito | `docs/metrics.md`, `docs/configuration.md` |
| Métricas: ingestão incremental de transcripts, custo, economia, projeção, latência | Feito | `docs/metrics.md` |
| Painéis coloridos no terminal (`stats`, `stats latency`, `metrics`) e relatório HTML offline | Feito | `docs/commands.md` |
| `audit` (contexto fixo e MCPs sem uso), `cache`, `pricing`, `update` com self-update verificado, `doctor --bench` | Feito | `docs/commands.md` |
| Pipeline de release (`dist`, Homebrew, Scoop, winget, crates.io) | Escrito, **nunca executado** | `docs/installation.md` |
| Qualidade | 655 testes; 100% de linhas cobertas; fmt, clippy, doc, deny e MSRV limpos no macOS | `docs/development.md` |

### 2.2 Problemas conhecidos (a corrigir antes do proxy)

1. **CI no Windows está vermelho.** O teste `ingestion_resumes_and_waits_for_partial_lines` (`tokred-metrics`) falha: com o arquivo aberto para escrita, o tamanho lido via metadados do diretório fica defasado no Windows. Além disso, o `cargo test --workspace` para no primeiro binário que falha, então **outras falhas do Windows podem estar escondidas** (caminhos, globs com `\`, testes de ponta a ponta). Correção: obter o tamanho pelo handle do arquivo (`File::open(..).metadata()`), rodar o CI com `--no-fail-fast` e tratar o que aparecer.
2. **Preços e famílias de modelo.** `Family` só conhece opus, sonnet e haiku; faltam Fable e Mythos. O multiplicador de leitura de cache é derivado como 0,1× quando falta no dado, o que está errado para modelos novos (0,05× e 0,025×). Falta o preço de escrita de 1 hora (2×).
3. **Pipeline de release não foi exercitado** (precisa de 4 secrets e de uma tag; o primeiro manifest do winget é manual).
4. **`route` usa palavras-chave** e não os cabeçalhos de classe da requisição (que só o proxy vê).

---

## 3. Princípios (inegociáveis)

1. **Nunca sobrescrever arquivos do usuário por inteiro** (só as partes do tokred). Idempotência de `configure`/`update`.
2. **Fail-open em tudo.** Hook: exit 0 sem saída. Proxy: qualquer erro em uma transformação ou decisão → repassar a requisição **original** intacta; panic é isolado por requisição; o daemon continua de pé. O tokred nunca pode impedir o Claude Code de funcionar por bug próprio.
3. **Latência.** Hook: p95 < 50 ms. Proxy: overhead p95 < 10 ms até o primeiro byte (medido, seção 8.4). Hooks nunca acessam a rede.
4. **Local e privado.** Nada sai da máquina além do que o Claude Code já envia à Anthropic. O proxy repassa **só** para o host configurado (padrão `https://api.anthropic.com`). Credenciais e corpos de requisição **nunca** vão para log nem para o banco (só hashes e contagens).
5. **Inspecionar sem modificar por padrão.** O proxy só altera uma requisição em casos explicitamente permitidos (seção 6) e sempre com saída de emergência. O modo padrão de qualquer recurso que altera tráfego é `observe`.
6. **Contexto injetado mínimo:** qualquer `additionalContext` ou `systemMessage` tem uma ou duas linhas e só aparece quando algo muda.
7. **Decisão baseada em dado.** Recurso que altera tráfego só é ligado depois que o modo `observe`/sombra mostrou ganho real e ausência de regressão (seção 12, portões).

---

## 4. Arquitetura

Arquitetura limpa no estilo idiomático de Rust: **regras de negócio puras no centro, I/O nas bordas**, dependências sempre para dentro e um único ponto de composição (o binário). As regras da v1 continuam valendo (lints estritos, `thiserror`, `anyhow` só no binário, `pub(crate)` por padrão, `tracing`, `&dyn` em vez de genéricos onde ajuda a cobertura).

### 4.1 Crates

```
crates/
├── tokred-core/      domínio puro + ports. NOVO: RequestClass, AgentType, TaskClass, Tier,
│                     ModelCapabilities, RoutingDecision, estimativa de custo com estado de cache
├── tokred-install/   configure/unconfigure/doctor, merge do settings, agentes
├── tokred-hooks/     handlers dos hooks (sincrono, sem tokio)
├── tokred-metrics/   ingestão de transcripts, custo, economia, projeção, latência
├── tokred-pricing/   preços, sync, snapshot embutido. NOVO: tabela de capacidades de modelos
├── tokred-audit/     auditoria do contexto fixo
├── tokred-update/    versão, método de instalação, self-update
├── tokred-report/    relatório HTML
├── tokred-store/     adapter SQLite, migrations, repositórios
├── tokred-proxy/     NOVO. Daemon HTTP assíncrono: repasse com streaming, tap de SSE,
│                     classificação, roteamento, medição, kill switch
└── tokred-cli/       binário `tokred`: composição, saída, comandos
```

### 4.2 Regras específicas do proxy

- **Fronteira assíncrona.** `tokio`/`hyper` vivem **só** em `tokred-proxy` e só iniciam no subcomando `proxy`. Hooks e demais comandos continuam síncronos e sem esse custo de inicialização.
- **Lógica pura no `tokred-core`/casos de uso:** `classify_request`, `decide_route`, `estimate_cost`, `explain_cache_miss`, `summarize_prompt`, `suggest_model` são funções puras testáveis sem rede. O `tokred-proxy` só faz I/O e chama essas funções.
- **Escritor único do SQLite.** O proxy atende requisições concorrentes, mas o SQLite tem um escritor: as requisições enviam registros por um canal (`mpsc`) a uma tarefa que grava em lote. Falha de gravação nunca afeta a resposta ao cliente.
- **Ports novos** em `tokred-core`: `ProxyRepo` (requisições, decisões, resumos de prompt, sugestões), `ModelCatalog` (capacidades, famílias, tiers), `UpstreamClient` (para falsos em teste).
- **Dependências novas** (só em `tokred-proxy`): `tokio`, `hyper` + `hyper-util` (ou `axum`), `reqwest` com rustls e streaming, `bytes`; em testes, `wiremock` e um servidor SSE falso próprio.

### 4.3 Paths

Inalterados em relação à v1 (pasta do Claude Code, pasta de dados via `BaseDirs`, banco, config, logs). Novo: `<dados>/proxy/` com o arquivo de segredo de administração (seção 6.8) e o PID.

---

## 5. Política de modelo

É o coração da v3. Princípio: **respeitar o que o usuário escolheu; decidir só o que ele não escolheu; sugerir o resto.**

### 5.1 Quem decide o quê

| Classe da requisição (`x-claude-code-request-class`) | O proxy pode trocar o modelo? | Como |
|---|---|---|
| `main` (loop principal) | **Nunca** (e nenhuma opção de configuração permite) | Só mede e **sugere** (seção 5.6) |
| `subagent`, `workflow` | Sim, **uma vez por agente** | Decisão no primeiro pedido de cada `agent-id` e fixa até o fim dele (seção 5.3) |
| `auxiliary` | Sim, **por requisição** e só se compensar | Cálculo de custo com estado de cache (seção 5.4) |
| `compaction` | Não por padrão | Qualidade do resumo afeta todo o resto da sessão; opt-in por configuração |

### 5.2 Camadas de classificação (primeira que decidir vence)

Cada camada devolve `(task_class, confidence, reason)`; o motor só age acima de um limiar de confiança (`min_confidence`). Em dúvida, **mantém o modelo pedido**.

1. **Cabeçalhos (determinístico).** `request-class` e `agent-type`. Ex.: `Explore` → busca (classe barata); `Plan` → planejamento (mantém); `auxiliary` → auxiliar.
2. **Escolha explícita do usuário.** Se o modelo do subagente **difere** do modelo que o loop principal da mesma sessão está usando, alguém fixou o modelo de propósito (frontmatter, variável, `opusplan`): **respeitar**. Só se decide quando o subagente **herdou** o modelo principal.
3. **Conjunto de ferramentas na requisição (objetivo).** Subagente só com ferramentas de leitura (`Read`, `Grep`, `Glob`, buscas) não altera nada: risco baixo para um modelo mais barato. Presença de `Edit`/`Write`/`Bash` sobe o risco.
4. **Palavras-chave no prompt da tarefa** (primeira mensagem do subagente), em pt-BR e en, reutilizando o classificador do `route` (movido para o `tokred-core`): buscar/localizar/resumir → barato; implementar/corrigir/refatorar → médio; arquitetura/design/planejar/revisar com profundidade → manter.
5. **Padrão:** manter.

### 5.3 Subagentes: decisão pegajosa

- A decisão é gravada por `agent-id` (`routing_decisions`) e **reutilizada** em todas as requisições seguintes desse agente. Nunca se reclassifica no meio da execução: isso quebraria o cache (cada subagente tem o seu) e a continuidade do thinking.
- Alvo: o tier definido para a classe (`[routing.tiers]`, seção 10), resolvido para um modelo concreto pelo catálogo (seção 7.3). **Só downgrade:** o alvo nunca pode custar mais do que o modelo pedido, nem estar fora de `allowed_models`.

### 5.4 Auxiliares: roteamento que considera custo

Algumas requisições auxiliares reaproveitam o cache da conversa (a sugestão de próximo prompt é "quase só leitura de cache"). Mover uma delas para o Haiku **perde o cache** e paga escrita a 1,25×, o que pode custar **mais** que a leitura a 0,1× no modelo caro. Por isso:

- Rotear só quando `estimate_cost(alvo, sem cache) < estimate_cost(original, estado de cache conhecido) × (1 − margem)`.
- Estado de cache conhecido: o proxy guarda, por sessão, o hash do prefixo principal e o instante da última leitura/escrita; se o prefixo da auxiliar coincide e o cache está quente, **não rotear**.
- Na prática: títulos, classificadores e resumos pequenos e independentes são roteados; sugestões que carregam o contexto inteiro, não.

### 5.5 Compatibilidade de parâmetros (obrigatória antes de trocar o modelo)

Modelos diferentes aceitam parâmetros diferentes (thinking adaptativo vs orçamento fixo, `effort`, interleaved thinking, `context_management`, contexto de 1M). Trocar o `model` sem ajustar o resto pode gerar `400`. Regras:

- O catálogo (seção 7.3) tem, por modelo, as capacidades. **Só se roteia** se todos os campos e betas presentes na requisição forem suportados pelo alvo. Caso contrário, **manter** o modelo (não se reescreve nenhum outro campo).
- **Rede de segurança:** se a requisição reescrita receber `400`/`404`/erro de modelo antes de qualquer byte de resposta ser enviado ao cliente, reenviar **uma vez** com o corpo e o modelo originais, registrar o motivo e contar a falha para o disjuntor.
- **Disjuntor (circuit breaker):** acima de `max_rewrite_failure_rate` (padrão 5% numa janela de 50 reescritas) o roteamento é desligado por `cooldown` e o `doctor` avisa. Também há chave manual (`tokred routing off`).

### 5.6 Loop principal: sugerir, nunca trocar

O modelo configurado pelo usuário é sempre respeitado. O tokred acompanha cada prompt do usuário e, quando a tarefa foi simples, **sugere** um modelo mais barato com a evidência resumida.

**Agregação.** Requisições com o mesmo `x-claude-code-prompt-id` formam um prompt. Para cada um o proxy grava (`prompt_summaries`): número de turnos principais, ferramentas usadas, tokens de saída e de thinking, custo real, se houve edição, subagentes disparados e duração das ferramentas.

**"Simples" (retrospectivo, transparente).** Critérios configuráveis e documentados, por exemplo: poucos turnos, saída pequena, thinking baixo, só leitura ou uma edição pequena. Retrospectivo é mais confiável que prever; a previsão para o próximo prompt usa só o padrão dos últimos N.

**Contrafactual.** Custo do mesmo prompt no tier mais barato apropriado, com o preço do catálogo e **o custo de reconstruir o cache** quando houver troca: `economia = custo_real − (custo_alvo + reescrita_do_cache)`. Só sugere se `economia > mínimo`.

**Quando e como avisar.** Nunca interromper o modelo. Três canais, todos opcionais (`[suggest] channel`):
1. `tokred stats`/`metrics`: seção "Sugestões".
2. **Status line** (`tokred statusline`): uma linha discreta ("simples nos últimos 5 prompts · Sonnet economizaria ~R$/US$ x"). O `configure` só grava a chave `statusLine` se estiver vazia e pergunta antes.
3. **Hook `UserPromptSubmit`** com `systemMessage` (mensagem para o usuário, não para o modelo), **só em fronteiras seguras**: cache já frio (ocioso além do TTL), logo após `/clear` ou compactação (troca de graça), no máximo uma vez por sessão e por janela de tempo.

O hook só **lê** a sugestão do banco (sem rede, dentro do orçamento de 50 ms).

### 5.7 Auditoria e transparência

- Toda decisão é gravada com motivo e `rules_version`. `tokred routing explain <request-id|agent-id>` mostra camada, confiança, estimativa de custo e resultado.
- Modo `observe` (padrão): registra "teria roteado X → Y" **sem alterar** nada, e mostra a economia hipotética. Só `enforce` altera tráfego.
- Cada recurso tem modo `off | observe | enforce`.

---

## 6. O proxy

Daemon HTTP local entre o Claude Code e a API. Opcional. Independente do `rtk-proxy`.

```
Claude Code ──(ANTHROPIC_BASE_URL=http://127.0.0.1:1029)──► tokred proxy ──► api.anthropic.com
                                                              │
                                                              ├─ lê cabeçalhos de classe/sessão/agente
                                                              ├─ classifica e (se enforce) escolhe o modelo
                                                              ├─ tap de SSE: mede uso sem atrasar a entrega
                                                              └─ grava requisição, decisão e prompt
```

### 6.1 Repasse

- Escuta só em loopback (`127.0.0.1`), porta padrão `1029`; recusa outro endereço sem `--bind` explícito. **Valida o cabeçalho `Host`** (defesa contra DNS rebinding) e responde `421` para hosts que não sejam loopback.
- Repassa só para `upstream` (HTTPS, rustls). Todos os cabeçalhos `anthropic-*`, `x-claude-code-*`, `Authorization`/`x-api-key` e os demais seguem **sem alteração** (sem lista fixa). Credenciais **nunca** vão para log ou banco.
- Só `POST /v1/messages` passa pela política. Todo outro endpoint (`count_tokens`, `/v1/models`, etc.) é repassado intacto.
- **Streaming:** a resposta é repassada evento por evento, sem bufferizar; o proxy observa os eventos em paralelo. Eventos `ping` e o cabeçalho `content-type: text/event-stream` seguem como vieram (o Claude Code tem um watchdog de ociosidade do stream). Erros da API (4xx, 5xx, 429) e seus corpos, e os cabeçalhos de rate limit, são repassados **exatamente**.
- O timeout do upstream é o do cliente; o proxy nunca o encurta.

### 6.2 Fail-open

Erro ou panic em classificação, decisão ou medição → repassar o corpo original e registrar. Erro do banco → ignorar (a resposta já foi entregue). Se o proxy estiver fora do ar, o `tokred claude` abre o Claude Code sem proxy e avisa.

### 6.3 Como o Claude Code aponta para o proxy

- **Padrão: `tokred claude [args…]`.** Verifica `GET /__tokred/health`, inicia o proxy se preciso e abre o Claude Code com `ANTHROPIC_BASE_URL` definido **só naquela execução**. Nada é gravado no `settings.json`.
- **Opcional: `persist_base_url = true`.** O `configure` grava `env.ANTHROPIC_BASE_URL` pelo merge da seção 5 da v1. Risco: com o proxy parado, o Claude Code inteiro para. Por isso exige o proxy como serviço do usuário (`tokred proxy service install`: systemd user unit, LaunchAgent, tarefa agendada no logon) e o `session-start` tenta subir o proxy. O `configure` explica o risco e pede confirmação. Se já houver `ANTHROPIC_BASE_URL` com outro valor, **não sobrescreve**: mostra e pergunta. O `unconfigure` remove só o valor do tokred.
- Autenticação: o proxy **não** substitui credenciais. Com chave de API (`x-api-key`) ou assinatura (`authorization`, verificado no spike F7), ele repassa o que recebe (a assinatura exige repassar o valor OAuth de `anthropic-beta`).

### 6.4 Medição (por requisição)

Grava em `proxy_requests`: modelo pedido e modelo efetivo, `usage` (input, output, cache write, cache read, thinking), classe, tipo e id do agente, id da sessão e do prompt, bytes do corpo, tempo até o primeiro byte do upstream, tempo total, **overhead do proxy** (tempo gasto antes de enviar), status, `routing_mode`, `routing_decision_id`, TTL de cache usado.

### 6.5 Explicação de cache miss

Para cada requisição `main` com `cache_read` baixo, o proxy explica o provável motivo comparando com a anterior da mesma sessão: **ociosidade acima do TTL** (intervalo entre requisições), **compactação** (cabeçalhos `compaction`/`context-compacted`), **mudança de ferramentas** (hash do bloco `tools`), **mudança do system** (hash do `system`), **mudança de efeito** (campos de thinking/effort), ou **desconhecido**. Só hashes de segmentos são guardados. O Claude Code também mostra um "likely cause" no `/usage`; o tokred cobre também subagentes e agrega ao longo do tempo.

### 6.6 Sombra de compressão e de cache de respostas

Sem alterar nada, o proxy calcula e registra:
- **Compressão (sombra):** aplicaria a compressão reversível da seção 6.7 (prévia + id, original guardado no banco) aos `tool_result` de texto e registra bytes e tokens que **teriam** sido evitados, por ferramenta. **Nunca há truncamento com perda:** todo conteúdo trocado continua recuperável pelo `tokred_retrieve`. Estimativa de ganho: um bloco de T tokens custa ≈ `1,25·T` na escrita e `0,1·T` por turno seguinte; o relatório mostra o ganho esperado já considerando a quantidade de turnos observada.
- **Cache de respostas (sombra):** hash canônico (blake3) do corpo para medir quantas requisições são **repetições exatas**. Só hashes e contagem.

### 6.7 Compressão reversível de resultados de ferramentas

**Princípio: nenhuma informação se perde.** O tokred nunca trunca conteúdo sem guardar o original; tudo o que sai da requisição pode ser recuperado.

Objetivo: o modelo não paga, a cada turno, por um `tool_result` enorme que já usou (ex.: um JSON de 5.000 linhas), mas consegue recuperá-lo quando precisar. Inspirado no mecanismo equivalente do `rtk-proxy`, com uma diferença deliberada na forma de recuperar.

**Como funciona**
1. **Seleção.** Só blocos `tool_result` de **texto** acima de `min_tokens` (padrão 1.500, estimados por bytes / 4). Nunca `Read` de arquivo de código pequeno, imagens, documentos, `tool_use`, `system`, `tools`, `thinking` nem `cache_control`.
2. **Troca determinística.** O proxy guarda o original (`stored_results`, blob comprimido, retenção `retention`, padrão 7 dias) e envia no lugar uma prévia (primeiras `preview_head_lines` e últimas `preview_tail_lines` linhas) seguida de um marcador em uma linha: `[tokred: N lines omitted (id=<id>). Call tokred_retrieve with this id and an optional offset/limit to read them.]`. O `id` é derivado do hash do conteúdo original.
3. **Mesmos bytes em todo turno.** A troca acontece **na primeira vez** que o bloco é enviado ao modelo (o turno logo depois de a ferramenta rodar), de modo que o modelo **nunca** viu o original nesse contexto. Nos turnos seguintes o Claude Code reenvia o original e o proxy reaplica exatamente a mesma troca (mapa `(sessão, hash do original) → bytes enviados`). Assim o prefixo do cache e os blocos de thinking (*preserved thinking*) continuam consistentes. Se o mapa disser outra coisa do que o cálculo atual, vale o mapa.
4. **Recuperação visível.** O `configure` registra um servidor MCP local (`tokred mcp`, stdio) com a ferramenta `tokred_retrieve(id, offset?, limit?)`. O modelo a chama como qualquer ferramenta, o Claude Code executa e a resposta entra no histórico normalmente.

**Por que não injetar uma ferramenta escondida (como o `rtk_retrieve`)**
- Atender a ferramenta dentro do proxy esconde uma troca do Claude Code. No turno seguinte o histórico reenviado não tem essa troca, e nos modelos com *preserved thinking* (Opus 5.5, Sonnet 5.5, Haiku 5.5, Fable 5.1) a API compara o prefixo e pode rejeitar com `400` ("bound to a different conversation") ou descartar blocos de thinking em silêncio.
- Injetar uma ferramenta muda o bloco `tools`, o que invalida o cache inteiro se acontecer no meio da sessão.
- Com o servidor MCP, a ferramenta existe desde o início da sessão (carregamento sob demanda das ferramentas MCP) e cada recuperação fica registrada no transcript.

**Salvaguardas**
- Modo `off | observe | enforce` (padrão `observe`: só mede quanto economizaria, seção 6.6). `enforce` só depois do portão G2.
- Se o proxy cair no meio da sessão, o Claude Code passa a enviar o original: uma reconstrução de cache e possível perda de blocos de thinking; o `doctor` e as métricas mostram isso.
- Recuperação do mesmo `id` mais de `max_retrievals` vezes numa sessão é sinal de que a prévia está curta demais para aquela ferramenta: a ferramenta passa a ser excluída automaticamente da compressão.
- Disjuntor: qualquer `400` de binding/thinking ou queda da taxa de cache depois de ativar desliga a compressão na hora e registra o motivo.
- **Extensão futura (não incluída):** trocar corpos repetidos de arquivo em argumentos de `Write`/`Edit` por um ponteiro (`retire-pointer` do `rtk-proxy`). Mexe em `tool_use` do assistente; avaliar depois de medir a F11.

### 6.8 Administração e segurança

- Endpoints `/__tokred/*` (health, status, parar) exigem um **segredo local** (cabeçalho com um valor aleatório gravado em `<dados>/proxy/secret`, permissão 0600) para impedir que páginas web ou outros processos controlem o daemon.
- `log_bodies = false` sempre; se ativado para depuração, avisa e nunca grava credenciais.
- O `doctor` mostra: proxy rodando, porta, serviço instalado, valor atual de `ANTHROPIC_BASE_URL`, modos de cada recurso, estado do disjuntor, versão do Claude Code vista nos cabeçalhos.

### 6.9 Compatibilidade

- O guia oficial diz que o gateway precisa acompanhar as versões do Claude Code. Por isso a regra é **repassar tudo o que não se entende**; só se age sobre o que é conhecido e testado.
- Matriz de compatibilidade testada e registrada: login por **chave de API** e por **assinatura**, `ANTHROPIC_BASE_URL` com e sem `ANTHROPIC_AUTH_TOKEN`, modelos com e sem preserved thinking, com e sem 1M de contexto.
- Conferir os termos de uso da Anthropic para tráfego de assinatura por proxy; se não for permitido, documentar o proxy como "somente chave de API". Registrar em `docs/decisions.md`.

---

## 7. Store e catálogo de modelos

### 7.1 Migração v3

Mantém todas as tabelas da v1/v2 do schema (`sessions`, `classifications`, `file_reads`, `hook_events`, `cache_entries`, `installed_files`, `pricing`, `usage`, `ingest_state`, `meta`). Novas:

```sql
proxy_requests(id TEXT PK, ts INT, session_id TEXT, agent_id TEXT NULL, agent_type TEXT NULL,
               prompt_id TEXT NULL, request_class TEXT, model_requested TEXT, model_effective TEXT,
               input INT, output INT, thinking INT, cache_write INT, cache_read INT, cache_ttl TEXT,
               bytes_in INT, upstream_ttfb_ms INT, total_ms INT, proxy_overhead_us INT,
               status INT, routing_mode TEXT, decision_id TEXT NULL, miss_cause TEXT NULL,
               prefix_hash TEXT, tools_hash TEXT, system_hash TEXT)
routing_decisions(id TEXT PK, ts INT, session_id TEXT, agent_id TEXT NULL, scope TEXT,   -- agent|request
                  task_class TEXT, confidence REAL, layer TEXT, reason TEXT,
                  model_from TEXT, model_to TEXT, mode TEXT, est_cost_from REAL, est_cost_to REAL,
                  rules_version TEXT, outcome TEXT)                                      -- applied|observed|kept|fallback
prompt_summaries(prompt_id TEXT PK, session_id TEXT, ts INT, turns INT, tools TEXT,
                 output_tokens INT, thinking_tokens INT, cost_usd REAL, edited INT,
                 subagents INT, simple INT, est_saving_usd REAL)
suggestions(id TEXT PK, ts INT, session_id TEXT, kind TEXT, message TEXT, shown INT, accepted INT NULL)
shadow_savings(ts INT, session_id TEXT, kind TEXT, key TEXT, bytes_saved INT, est_tokens_saved INT, rules_version TEXT)
compressed_blocks(...)   -- só se a compressão ativa for ligada (F10+)
```

Índices por `ts`, `session_id`, `prompt_id`, `agent_id`. Retenção configurável (`[proxy] retention_days`, padrão 90) com limpeza oportunista nos comandos da CLI.

### 7.2 Fontes de métricas

Quando há dados do proxy no período, eles são a fonte principal de uso e custo; os transcripts completam o que o proxy não viu. Deduplicar pelo id da mensagem. O `stats`/`metrics` ganham agrupamentos por classe de requisição, tipo de agente e prompt.

### 7.3 Catálogo de modelos (dados, não código)

Arquivo embutido e atualizável (`models.json`, gerado por script a partir do LiteLLM e da documentação, com fallback offline) com, por modelo: família, preços (input, output, escrita 5 min, escrita 1 h, leitura de cache), **capacidades** (thinking adaptativo/orçamento, `effort`, interleaved, `context_management`, 1M de contexto, mínimo cacheável), e os **tiers** padrão (barato/médio/forte) por família. `Family` deixa de ser um enum fechado (passa a aceitar famílias desconhecidas). Modelo desconhecido: família inferida pelo nome, capacidades **conservadoras** (não rotear) e aviso no `stats`.

---

## 8. Mudanças nos componentes existentes

| Componente | Mudança |
|---|---|
| `tokred-core` | Tipos do proxy e da política; `Family` aberta; `ModelPrice` com escrita de 1 h e multiplicadores por modelo; `estimate_cost` com estado de cache; mover o classificador de palavras-chave do `tokred-hooks` para cá |
| `tokred-pricing` | Ler o preço de 1 h e os multiplicadores reais do LiteLLM (sem derivar 0,1× quando houver dado); catálogo de capacidades; script de atualização |
| `tokred-hooks` | `route` vira sugestão opcional; novo comportamento de sugestão no `UserPromptSubmit` (lê do banco, só em fronteiras seguras); `session-start` tenta subir o proxy quando configurado; guardas inalteradas |
| `tokred-metrics` | Fonte proxy + transcripts; custo por classe/agente/prompt; economia **real** do roteamento (separada do contrafactual "tudo no opus"); explicações de cache miss; economia de sombra; taxa de sugestões aceitas |
| `tokred-report` | Seções novas: proxy (overhead p50/p95, orçamento 10 ms), decisões de roteamento, sugestões, causas de cache miss, economia de sombra |
| `tokred-store` | Migração v3 e repositórios do proxy; escritor único em lote |
| `tokred-install` | `configure` pergunta sobre o proxy, `statusLine` e `ANTHROPIC_BASE_URL` (merge não destrutivo, nunca sobrescreve valor alheio); `unconfigure` remove só o que é do tokred |
| `tokred-update` | Depois de atualizar o binário, reiniciar o daemon do proxy; compatibilidade de schema |
| `tokred-cli` | Comandos novos (seção 9); `doctor` com a seção do proxy; `stats --by request-class\|agent-type\|prompt` |
| CI/Release | Matriz de 3 SOs também para o proxy (testes com API falsa e streaming); `--no-fail-fast`; instaladores de serviço de usuário por SO |

---

## 9. Comandos (novos e alterados)

| Comando | Função |
|---|---|
| `tokred proxy start\|stop\|status` | Controla o daemon (`start` em primeiro plano ou em segundo plano) |
| `tokred proxy service install\|uninstall` | Serviço do usuário (systemd user unit, LaunchAgent, tarefa de logon) |
| `tokred claude [args…]` | Garante o proxy e abre o Claude Code apontado para ele só nessa execução |
| `tokred routing status\|on\|off\|explain <id>` | Modo, disjuntor, últimas decisões e explicação de uma decisão |
| `tokred suggest [--since 7d]` | Sugestões de modelo para o loop principal, com a evidência |
| `tokred statusline` | Linha curta para o `statusLine` do Claude Code |
| `tokred mcp` | **Interno.** Servidor MCP (stdio) com a ferramenta `tokred_retrieve`, registrado pelo `configure` quando a compressão está habilitada (seção 6.7) |
| `tokred stats … --by request-class\|agent-type\|prompt` | Novos agrupamentos |
| `tokred doctor` | Seção do proxy (seção 6.8) |

Os demais comandos (`configure`, `unconfigure`, `update`, `audit`, `pricing`, `cache`, `metrics`, `stats`, `hook`) seguem `docs/commands.md`.

---

## 10. Config (`tokred.toml`) — chaves novas

```toml
[proxy]
enabled = false
bind = "127.0.0.1:1029"
upstream = "https://api.anthropic.com"
persist_base_url = false         # grava env.ANTHROPIC_BASE_URL no settings (exige serviço)
retention_days = 90
log_bodies = false

[routing]
mode = "observe"                 # off | observe | enforce
min_confidence = 0.8
min_saving_usd = 0.005
max_rewrite_failure_rate = 0.05
cooldown = "30m"
allowed_models = []              # vazio = qualquer modelo do catálogo que seja mais barato
compaction = "keep"              # keep | cheap (opt-in)

[routing.tiers]                  # classe da tarefa -> tier; o catálogo resolve para um modelo concreto
search = "cheap"
summary = "cheap"
implementation = "mid"
planning = "keep"
architecture = "keep"
review = "keep"

[suggest]
channel = "stats"                # off | stats | statusline | message
min_simple_prompts = 3
cooldown = "6h"

[compression]                    # compressão reversível (seção 6.7)
mode = "observe"                 # off | observe | enforce
min_tokens = 1500
preview_head_lines = 40
preview_tail_lines = 20
retention = "7d"
max_retrievals = 3

[shadow]
compression = true               # calcula o que economizaria (não altera nada)
response_cache = true            # mede repetições exatas por hash
```

As chaves de hooks, preços, update, cache e guardas da v1 continuam (veja `docs/configuration.md`). A config do usuário continua passando por merge no update.

---

## 11. Testes (proxy)

| Tipo | O que cobre |
|---|---|
| **Unitários** | Classificação em camadas (tabela de casos pt-BR/en e cabeçalhos), decisão de rota, compatibilidade de parâmetros, estimativa de custo com e sem cache, explicação de cache miss, agregação de prompt, regras de "simples" |
| **API falsa** | Servidor Anthropic falso (`wiremock` + SSE próprio): repasse byte a byte, streaming evento por evento, erros 4xx/5xx/429 intactos, queda do upstream no meio do stream, cabeçalhos preservados (inclusive desconhecidos) |
| **Roteamento** | Decisão pegajosa por `agent-id`; downgrade apenas; escolha explícita respeitada; fallback com reenvio do original em `400`; disjuntor abre e fecha; modo `observe` nunca altera um byte |
| **Propriedade** | Em `observe`/sombra o corpo enviado ao upstream é **idêntico** ao recebido; mesma entrada → mesma decisão |
| **Carga e latência** | Overhead p95 < 10 ms até o primeiro byte; 50 streams simultâneos; sem vazamento de memória |
| **Ponta a ponta** | `tokred claude` com um "claude" falso; sessão simulada de vários turnos |
| **3 SOs** | Todos acima no CI (inclusive caminhos do Windows) |
| **Manuais, registrados** | Claude Code real com chave de API e com assinatura (spike, F6); Claude Code real com roteamento em `enforce` em sessões de teste |

Regras da v1 mantidas: nenhum teste toca o `~/.claude` nem a pasta de dados reais; nenhum acessa a rede externa; toda correção de bug entra com um teste que a reproduz; **cobertura de linhas em 100%** (medida com `cargo llvm-cov`, referência: `lcov` e o texto anotado).

---

## 12. Fases

Cada fase termina com testes verdes nos 3 SOs, clippy limpo, cobertura mantida e documentação nos dois idiomas. Os **portões** são decisões baseadas em dados, registradas em `docs/decisions.md`.

| Fase | Entrega | Pronto quando |
|---|---|---|
| **F0 a F5** | Feito (seção 2) | — |
| **F6 — Estabilização** | Corrigir o CI do Windows (tamanho pelo handle; `--no-fail-fast`; tratar o que mais aparecer); catálogo de modelos e preços (famílias novas, multiplicadores, 1 h); exercitar o pipeline de release com uma tag de teste | CI verde nos 3 SOs; `stats` correto para modelos novos; release de teste publicada |
| **F7 — Spike (≈ 1 dia)** | Proxy mínimo de repasse que só registra cabeçalhos e `usage`; testar com **chave de API** e com **assinatura**; confirmar que os cabeçalhos `x-claude-code-*` chegam; testar reescrever o `model` de uma requisição auxiliar | Resultado em `docs/decisions.md`: o que funciona, o que não; **go/no-go** do proxy e se assinatura é suportada |
| **F8 — Proxy: repasse e medição** | `tokred-proxy` (6.1 a 6.5, 6.8), `tokred claude`, `proxy start/stop/status/service`, migração v3, explicação de cache miss, integração nas métricas e no `doctor` | Testes com API falsa e streaming; overhead p95 < 10 ms; Claude Code normal com e sem o proxy; nenhuma alteração de byte no tráfego |
| **F9 — Política de modelo** | Classificação em camadas, roteamento de subagentes e auxiliares em modo `observe` → `enforce`, catálogo de capacidades, fallback e disjuntor, `routing explain` | Em `observe`, economia hipotética visível; **portão G1:** só ligar `enforce` se a taxa de decisões corretas revisadas for alta e o `observe` não tiver mostrado perdas de qualidade; em `enforce`, 0 `400` de binding/thinking nas sessões de teste |
| **F10 — Sugestões e sombra** | `prompt_summaries`, `suggest`, `statusline`, hook de sugestão em fronteiras seguras; sombra de compressão e de cache de respostas; relatório de ganho | Sugestões com evidência correta nos testes; **portão G2:** compressão ativa só se a sombra mostrar ganho material (limiar definido nos dados) **e** um canário sem `400` nem queda de cache |
| **F11 — Compressão reversível** | Seção 6.7: troca determinística + `stored_results` + servidor MCP `tokred mcp` com `tokred_retrieve`, em `observe` por padrão; registro do MCP pelo `configure`; cache de respostas ativo continua no backlog | Testes de determinismo e de prefixo byte a byte entre turnos; recuperação ponta a ponta com o Claude Code real; **portão G2** para `enforce`: ganho medido na sombra e nenhum `400` nem queda de cache num canário |
| **Backlog** | Cache de artefatos (resumos por hash) e demais ideias da v1 | Só com dados |

---

## 13. Riscos e mitigações

| Risco | Mitigação |
|---|---|
| A API ou o Claude Code mudam (novos betas, campos, cabeçalhos) | Repassar tudo o que não se entende; agir só no conhecido e testado; matriz de compatibilidade; disjuntor; `doctor` mostra a versão do Claude Code |
| Rotear degrada a qualidade da resposta | Só tarefas de baixo risco; subagentes herdados; `observe` por padrão; limiar de confiança; só downgrade; `routing off` imediato; revisão por amostragem em F9 |
| `400` por parâmetros incompatíveis entre modelos | Catálogo de capacidades; só rotear se compatível; reenvio do original; disjuntor |
| Perder cache ao trocar de modelo | Decisão pegajosa por agente; auxiliares só quando compensa; nunca no loop principal |
| Preserved thinking / binding | Nunca alterar `system`, `tools`, `messages` em `observe`; compressão só em sombra até o portão |
| Proxy vira ponto único de falha | Fail-open; `tokred claude` sem proxy se não subir; serviço só com opt-in explícito |
| Segurança do daemon local | Loopback, validação do `Host`, segredo de administração, sem log de credenciais ou corpos |
| Termos de uso da assinatura via proxy | Verificar e registrar no spike; se não permitido, "somente chave de API" |
| Custo de manutenção do proxy | Escopo mínimo, dados em vez de código (catálogo), testes com API falsa, matriz no CI |
| Ganho menor que o esperado | Medir antes (F8), sombra antes de agir (F10), portões G1/G2 com limiares nos dados |
| Compressão reversível quebra cache ou thinking | Troca na primeira vez que o bloco é enviado e idêntica depois; recuperação por ferramenta MCP visível (sem troca escondida); disjuntor; `observe` por padrão |
| A prévia não basta e o modelo não recupera | Marcador explícito com o id e a ferramenta; exclusão automática da ferramenta quando há recuperações repetidas; limiar alto (`min_tokens`) |

### 13.1 Perguntas em aberto (resolver no spike F7)

1. Tráfego de **assinatura** tolera a reescrita do campo `model` (planos têm limites por família)? E é permitido pelos termos? **Respondido tecnicamente (spike F7):** o login por assinatura aceita o `model` reescrito nas requisições de subagente (status 200, resultado correto). **Ainda em aberto:** os termos de uso da Anthropic para tráfego de assinatura por proxy e os limites do plano por modelo.
2. O Claude Code reage à resposta com `model` diferente do pedido (interface, contabilidade interna)? **Respondido para chave de API:** a API aceitou e o Claude Code não teve problema (spike F7).
3. Os cabeçalhos `x-claude-code-*` aparecem em todas as superfícies (CLI, extensões)? **Respondido para a CLI:** nos modos `-p` e interativo só `session-id` e `agent-id`.
4. Quais requisições `auxiliary` compartilham o prefixo do cache na prática (medir)?

---

## 14. Distribuição e documentação

- **Distribuição:** inalterada (`dist`, Homebrew, Scoop, winget, crates.io; instalar e configurar são passos separados). Novo: o proxy como serviço do usuário é sempre **opt-in**; nenhum canal de instalação ativa o proxy.
- **Documentação:** continua bilíngue. Novos: `docs/proxy.md` (como funciona, segurança, compatibilidade), `docs/routing.md` (política de modelo e como auditar decisões), e atualizações de `commands`, `configuration`, `hooks`, `metrics`, `architecture`, `installation` e `decisions`. Exemplos de comandos devem funcionar como escritos; os da seção de início rápido têm teste de ponta a ponta.
- **Privacidade (README):** tudo local; as únicas chamadas de rede do tokred são a sincronização de preços, a verificação de versão e `update` (só a partir de comandos da CLI) e o repasse do proxy ao host da API; `TOKRED_OFFLINE=1` desliga as três primeiras.
