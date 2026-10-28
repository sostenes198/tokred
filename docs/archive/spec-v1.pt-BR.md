> **Arquivado.** Substituída pela especificação atual: [spec.pt-BR.md](../spec.pt-BR.md).

[English](spec-v1.md) | **Português**

# tokred — Especificação de implementação

**Repositório:** https://github.com/sostenes198/tokred

**Descrição curta (GitHub "About"):** *Token and cost reducer for Claude Code — model routing, context guards and local metrics. Works alongside rtk.*

**Descrição:** tokred é uma CLI em Rust, local e multiplataforma, que reduz o custo e o consumo de tokens do Claude Code. Ela instala subagentes com o modelo certo para cada tipo de tarefa (opus para arquitetura e planejamento, sonnet para desenvolvimento, haiku para busca e resumo), hooks que reforçam essa delegação e bloqueiam leituras inúteis de contexto, e um painel de métricas com tokens, custo, economia e latência adicionada. Tudo roda na máquina do usuário, com um SQLite próprio, sem serviço externo. Foi pensada para rodar junto com o rtk (rust token killer), que comprime a saída de comandos; o tokred cuida do resto.

## 0. Instruções para o Claude Code

- Antes de implementar **cada hook**, consulte a documentação oficial de hooks do Claude Code (https://code.claude.com/docs/en/hooks). Confira o formato de entrada (stdin), o de saída (`hookSpecificOutput`, `permissionDecision`, `additionalContext`, `systemMessage`) e o nome atual da ferramenta de subagentes (`Task` ou `Agent`). **Não assuma nomes de campo de memória.** Onde esta spec diverge da doc, a doc vence: registre a divergência em `docs/DECISIONS.md`.
- Faça a implementação por fases (seção 12). Ao final de cada fase: testes passando, `cargo clippy -- -D warnings` limpo, e um resumo do que foi feito.
- Pergunte antes de qualquer decisão que mude o comportamento descrito aqui.

### Regras de idioma

- **Código em inglês:** nomes de pastas, arquivos, crates, módulos, tipos, funções, variáveis, tabelas e colunas do banco, chaves da config, comentários, mensagens de commit e todas as strings exibidas pela CLI e pelos hooks (incluindo o `additionalContext`, os avisos e o HTML de métricas).
- **Templates dos subagentes em inglês.** As palavras-chave do classificador continuam em pt-BR e en, porque são dados para reconhecer prompts nos dois idiomas.
- **Documentação sempre nos dois idiomas:** cada documento tem a versão em inglês e a versão em português, com o mesmo conteúdo. Convenção: `README.md` + `README.pt-BR.md`, `docs/<nome>.md` + `docs/<nome>.pt-BR.md`. Isso vale para README, DECISIONS, CHANGELOG, guias e esta spec (salvar esta como `docs/spec.pt-BR.md` e criar a tradução `docs/spec.md`). Ao alterar um, atualizar o outro no mesmo commit.
- Os exemplos de mensagens nesta spec já estão em inglês; os nomes em português aqui são só explicação.

---

## 1. Visão geral

tokred é um binário único em Rust, multiplataforma (Windows, Linux e macOS). Ele roda **junto com o rtk** (rust token killer) e reduz custo e consumo de tokens no Claude Code:

1. **Roteamento de modelo:** instala subagentes com modelo fixo (opus, sonnet ou haiku) e hooks que sugerem e reforçam a delegação correta.
2. **Guarda de contexto:** hooks que impedem leituras redundantes ou inúteis (releitura de arquivo sem mudança, lockfiles, arquivos gerados, arquivos enormes sem `offset`/`limit`).
3. **Auditoria:** mede o peso do contexto fixo (CLAUDE.md, skills, agentes, MCPs configurados × uso).
4. **Métricas:** lê os transcripts do Claude Code e mostra tokens, custo, economia por roteamento, economia pelas guardas e projeção.
5. **Store local:** SQLite na pasta do tokred, com cache em camadas (L1/L2/L3) para estado de sessão e artefatos.

### Não-objetivos (por enquanto)

- **Cache de respostas do LLM / proxy HTTP:** fica fora do MVP. Hooks não interceptam chamadas à API, prompts agênticos quase nunca se repetem byte a byte, e a Anthropic já faz cache de prefixo no servidor. Será reavaliado na fase 7, com base nos dados das métricas.
- Comprimir saída de comandos Bash: isso é papel do rtk.
- Redis ou qualquer serviço externo.

---

## 2. Princípios (inegociáveis)

1. **Nunca sobrescrever arquivos do usuário por inteiro.** O tokred só adiciona, atualiza ou remove **as partes dele** (seção 5).
2. **Idempotência:** rodar `configure` ou `update` N vezes produz o mesmo resultado.
3. **Fail-open:** se um hook der erro, panic ou timeout, ele **sai com código 0 e sem saída**, e loga o erro em arquivo. O tokred nunca pode travar ou bloquear o Claude Code por bug próprio. Use `std::panic::catch_unwind` no entrypoint dos hooks.
4. **Latência:** cada execução de hook deve ficar abaixo de ~50 ms (p95). Nada de rede dentro de hook.
5. **Local e privado:** nenhum dado sai da máquina. As únicas chamadas de rede são a sincronização de preços (seção 9) e a verificação de versão (seção 14), feitas só a partir de comandos da CLI, nunca em hooks.
6. **Contexto injetado mínimo:** qualquer `additionalContext` tem no máximo uma ou duas linhas, e só é emitido quando muda algo.

---

## 3. Arquitetura

Arquitetura limpa no estilo idiomático de Rust: **regras de negócio puras no centro, I/O nas bordas**, dependências apontando sempre para dentro, e um único ponto de composição (o binário).

### 3.1 Layout do repositório

```
tokred/
├── Cargo.toml                    workspace: members, [workspace.package], [workspace.dependencies], [workspace.lints]
├── Cargo.lock                    versionado (projeto com binário)
├── rust-toolchain.toml           canal stable + rustfmt + clippy
├── rustfmt.toml
├── clippy.toml
├── deny.toml                     cargo-deny: licenças, advisories, fontes
├── dist-workspace.toml           configuração do dist (releases)
├── .editorconfig
├── .gitattributes                normalização de fim de linha (LF; *.ps1 com CRLF)
├── .gitignore
├── LICENSE-MIT / LICENSE-APACHE
├── README.md / README.pt-BR.md
├── CHANGELOG.md / CHANGELOG.pt-BR.md
├── CONTRIBUTING.md / CONTRIBUTING.pt-BR.md
├── .github/
│   ├── workflows/ci.yml          fmt, clippy, testes (3 SOs), cargo-deny, cobertura
│   ├── workflows/release.yml     gerado pelo dist + publicação no winget e no scoop
│   ├── dependabot.yml            cargo + github-actions
│   ├── ISSUE_TEMPLATE/           bug e feature
│   └── pull_request_template.md
├── crates/
│   ├── tokred-core/              domínio: tipos, erros, modelo de config, resolução de paths, traits (ports)
│   ├── tokred-install/           caso de uso: configure/unconfigure, merge do settings, agentes, manifesto
│   ├── tokred-hooks/             caso de uso: handlers dos hooks (route, enforce, guard-read, guard-search, session-*)
│   ├── tokred-metrics/           caso de uso: ingestão de transcripts, custo, economia, projeção, latência
│   ├── tokred-pricing/           caso de uso: tabela de preços, sync, fallback embutido
│   ├── tokred-audit/             caso de uso: auditoria do contexto fixo
│   ├── tokred-update/            caso de uso: verificação de versão, detecção do método de instalação, self-update
│   ├── tokred-report/            geração do HTML de métricas (template + SVG)
│   ├── tokred-store/             adapter: SQLite (rusqlite), migrations, implementação dos repositórios
│   └── tokred-cli/               binário `tokred`: clap, composição dos adapters, saída (tabela/JSON)
├── templates/agents/             templates .md dos subagentes (inglês)
├── tests/fixtures/               settings.json reais (com hooks do rtk), stdin de hooks, transcripts JSONL
└── docs/                         documentação bilíngue (seção 16)
```

### 3.2 Camadas e regras de dependência

```
tokred-cli  ──►  casos de uso (install, hooks, metrics, pricing, audit, update, report)  ──►  tokred-core
     │                                                                                          ▲
     └──►  adapters (tokred-store, módulos http/fs)  ── implementam os traits de ───────────────┘
```

- **`tokred-core`** não faz I/O: tipos de domínio, erros (`thiserror`), modelo de config, e os **traits (ports)** que os casos de uso consomem: `Clock`, `FileSystem`, `SessionRepo`, `UsageRepo`, `CacheRepo`, `PriceSource`, `ReleaseSource`, `HookEventSink` etc.
- **Casos de uso** dependem só de `tokred-core`. A lógica principal é feita de **funções puras** sobre dados, por exemplo:
  - `merge_settings(current: Value, entries: &[HookEntry]) -> Result<Value, MergeError>`
  - `classify(prompt: &str, rules: &Rules) -> TaskClass`
  - `next_tier(entry: &CacheEntry, now: Timestamp, policy: &TierPolicy) -> Tier`
  - `cost(usage: &Usage, prices: &PriceTable) -> Money`
  
  O I/O entra só pelos traits, recebidos por parâmetro (genéricos ou `&dyn Trait`).
- **Adapters** (`tokred-store`, e os módulos `http` e `fs` dentro dos crates que precisam) implementam os traits. Nenhum caso de uso importa `rusqlite`, `ureq` ou `std::fs` diretamente.
- **`tokred-cli`** é o único ponto de composição: monta os adapters reais, chama os casos de uso, formata a saída e decide o código de saída. Só ele usa `anyhow`.
- `Clock` existe para que TTLs do cache, a janela de 24 h da verificação de versão e as métricas por dia sejam testáveis sem esperar o tempo passar.

### 3.3 Padrões de código

- Edição Rust 2024, `rust-version` (MSRV) declarado no workspace e verificado no CI.
- Versões das dependências centralizadas em `[workspace.dependencies]`; os crates usam `dep = { workspace = true }`.
- Lints em `[workspace.lints]`: `unsafe_code = "forbid"`; clippy `pedantic` como `warn`; `unwrap_used`, `expect_used`, `panic`, `todo` e `dbg_macro` como `deny` fora de testes.
- Erros: `thiserror` com enums específicos por crate; nada de `String` como erro; `anyhow` só no binário.
- Itens públicos com doc comments (`///`), e cada crate com doc de módulo (`//!`) explicando o papel dele na arquitetura. `cargo doc --no-deps` sem warnings.
- API pública mínima: `pub(crate)` por padrão, só expõe o que outro crate usa.
- Logs com `tracing`; nunca `println!` fora do `tokred-cli`.
- Formatação com `cargo fmt`; commits no padrão Conventional Commits (`feat:`, `fix:`, `docs:`...), em inglês.

### 3.4 Testes

| Tipo | Onde | O que cobre |
|---|---|---|
| **Unitários** | `#[cfg(test)] mod tests` dentro de cada módulo | Funções puras: merge, classificador, política de camadas do cache, cálculo de custo e projeção, detecção do método de instalação, parsing de transcripts |
| **Integração por crate** | `crates/<crate>/tests/` | Caso de uso com adapters reais em diretório temporário: `tokred-store` com SQLite real em `tempfile`, `tokred-install` contra fixtures de `settings.json`, `tokred-metrics` contra transcripts de fixture |
| **Ponta a ponta (CLI)** | `crates/tokred-cli/tests/` | Executa o binário real com `assert_cmd` + `predicates`: `configure` → hooks com stdin de fixture → `stats`/`metrics` → `unconfigure`, tudo dentro de um `--claude-dir` e de uma pasta de dados temporários |
| **Snapshot** | junto dos testes acima | `insta` para o `settings.json` resultante do merge, a saída `--json` dos comandos e o HTML de métricas |
| **Propriedade** | testes do merge | `proptest`: configure duas vezes = configure uma vez; configure + unconfigure = original |

Regras:
- **Nenhum teste toca o `~/.claude` ou a pasta de dados real.** A variável `TOKRED_DATA_DIR` e a flag `--claude-dir` existem também para isso; os testes sempre usam diretórios temporários.
- Nenhum teste acessa a rede: `PriceSource` e `ReleaseSource` têm implementações falsas para teste.
- Testes rodando nos 3 SOs no CI, incluindo paths do Windows com espaço.
- Cobertura medida com `cargo llvm-cov`; meta de pelo menos 80% nos crates de caso de uso e no `tokred-core`.
- Toda correção de bug entra com um teste que reproduz o bug.

### 3.5 Crates sugeridas

`clap` (derive), `serde`, `serde_json` (feature **`preserve_order`**), `toml`, `rusqlite` (feature **`bundled`**), `directories`, `blake3`, `globset`, `walkdir`, `thiserror`, `anyhow` (só no binário), `ureq` com rustls (só nos adapters http), `comfy-table`, `time`, `opener`, `tracing` + `tracing-appender`.

Para testes: `tempfile`, `assert_cmd`, `predicates`, `insta`, `proptest`.

### Paths

| Item | Origem |
|---|---|
| Pasta do Claude Code | `$CLAUDE_CONFIG_DIR` se definido; senão `~/.claude` (no Windows, `%USERPROFILE%\.claude`) |
| Pasta de dados do tokred | `TOKRED_DATA_DIR` se definido; senão `directories::ProjectDirs` → Linux `~/.local/share/tokred`, macOS `~/Library/Application Support/tokred`, Windows `%APPDATA%\tokred` |
| Binário | Instalado pelo gerenciador de pacotes ou script (seção 14). O tokred não copia o próprio binário |
| Banco | `<dados>/tokred.db` |
| Config | `<dados>/tokred.toml` |
| Logs | `<dados>/logs/` (rotação diária, manter 7 dias) |

---

## 4. Comandos da CLI

| Comando | Função |
|---|---|
| `tokred configure [--claude-dir <path>] [--yes]` | Detecta a pasta do Claude Code (ou pergunta), cria o banco e a config, faz o merge do settings, instala os agentes e sincroniza os preços (seção 5) |
| `tokred update` | Atualiza o binário de acordo com o método de instalação (seção 14) e reaplica a configuração de forma idempotente |
| `tokred unconfigure [--purge]` | Remove só as partes do tokred do Claude Code; `--purge` também apaga a pasta de dados. O binário em si é removido pelo gerenciador de pacotes |
| `tokred doctor [--bench]` | Mostra o que está instalado, versões, arquivos desatualizados, conflitos com outros hooks e se o rtk foi detectado; `--bench` mede a latência real de cada hook (seção 10) |
| `tokred stats [--since 7d] [--by model\|day\|project]` | Tokens, custo real, custo contrafactual, economia e projeção |
| `tokred stats latency [--since 7d] [--by hook\|day\|session]` | Quanto tempo os hooks do tokred acrescentaram ao Claude Code |
| `tokred metrics [--since 30d] [--no-open]` | Gera o relatório HTML visual (seção 10) sempre no mesmo arquivo e abre no navegador |
| `tokred audit [--project <path>]` | Peso estimado do contexto fixo e MCPs sem uso |
| `tokred pricing sync` / `pricing show` | Atualiza ou mostra a tabela de preços |
| `tokred cache stats` / `cache prune` | Estado do cache por camada e limpeza de expirados |
| `tokred hook <evento>` | **Interno**, chamado pelo Claude Code. Lê JSON do stdin e escreve JSON no stdout |

Todos os comandos de visualização aceitam `--json` para saída legível por máquina.

---

## 5. `tokred configure`: detecção e merge não destrutivo

Instalar o binário (seção 14) **não mexe em nada** do Claude Code. A integração só acontece quando o usuário roda `tokred configure`.

### 5.0 Fluxo do `configure`

1. **Encontrar a pasta do Claude Code**, nesta ordem:
   1. flag `--claude-dir <path>`;
   2. valor salvo em `tokred.toml` (`[claude] dir`) de uma configuração anterior;
   3. variável `CLAUDE_CONFIG_DIR`;
   4. padrão `~/.claude` (Windows: `%USERPROFILE%\.claude`).
   
   Uma pasta é válida se existe e tem `settings.json` ou `projects/`.
2. **Se não encontrar:** em terminal interativo, pede o path, valida e pergunta de novo se for inválido. Sem terminal interativo (CI, script), sai com erro sugerindo `--claude-dir`.
3. Salva o path escolhido em `tokred.toml`.
4. Detecta o rtk e informa que os dois vão coexistir.
5. **Mostra o plano** antes de aplicar: hooks que serão adicionados ou atualizados no `settings.json`, agentes que serão criados ou atualizados, arquivos `.new` por conflito. Pede confirmação; `--yes` pula a pergunta.
6. Cria a pasta de dados, o banco (migrations) e a config (merge, seção 13), aplica o merge do settings (5.2), instala os agentes (5.3) e sincroniza os preços (se falhar, usa o fallback embutido).
7. Termina mostrando os próximos passos (`tokred doctor`, `tokred metrics`).

### 5.1 Assinatura

Toda entrada de hook do tokred tem um `command` que contém o marcador **`tokred hook `**. Exemplo:

```json
{ "type": "command", "command": "\"/opt/homebrew/bin/tokred\" hook guard-read", "timeout": 5 }
```

- O path do binário vai sempre entre aspas, porque no Windows ele pode ter espaços.
- **Usar um path estável**, que sobreviva a upgrades: o path pelo qual o binário é encontrado no `PATH` (ex.: `/opt/homebrew/bin/tokred`, o shim do scoop, o link do winget), e **não** o path canonicalizado com a versão no nome (ex.: `.../Cellar/tokred/0.3.2/bin/tokred`), que quebra no próximo upgrade. Procurar `tokred` no `PATH` e conferir que aponta para o executável atual; se não achar, usar `current_exe()` e avisar no `doctor`.
- O `doctor` confere se o path gravado no settings ainda existe; o `configure` corrige.

### 5.2 Algoritmo de merge do `settings.json`

1. Se o arquivo não existe, criar `{}`.
2. Ler e fazer parse. **Se o JSON for inválido: abortar com erro claro, sem escrever nada.**
3. Fazer backup em `settings.json.tokred.bak` (manter só o último).
4. Para cada evento em `hooks`: percorrer os grupos (matcher + `hooks[]`) e **remover só as entradas cujo `command` contém o marcador**. Se um grupo ficar com `hooks` vazio, remover o grupo. Se o evento ficar vazio, remover o evento.
5. Inserir as entradas atuais do tokred (seção 7), em grupos próprios. **Não misturar no mesmo grupo dos hooks do rtk ou do usuário.**
6. Todas as outras chaves (`permissions`, `env`, `model`, hooks de terceiros etc.) ficam intactas e **na ordem original**.
7. Gravar num arquivo temporário no mesmo diretório e depois fazer rename atômico.

O `unconfigure` é o mesmo algoritmo, sem o passo 5.

### 5.3 Agentes

- Os arquivos ficam em `<claude>/agents/`, com prefixo `tokred-` (ex.: `tokred-architect.md`).
- A primeira linha do corpo, logo depois do frontmatter, é o comentário `<!-- managed-by: tokred vX.Y.Z -->`.
- O manifesto (tabela `installed_files`) guarda o path, o hash blake3 do conteúdo instalado e a versão.
- No update, se o hash atual do arquivo **for igual** ao do manifesto, sobrescreve. Se **for diferente** (o usuário editou), não sobrescreve: grava `tokred-x.md.new` ao lado e avisa no `doctor`. A flag `--force` sobrescreve mesmo assim.
- Arquivos sem o prefixo e sem o marcador **nunca** são tocados.

### 5.4 Testes obrigatórios do merge

Fixtures com: settings vazio; settings com hooks do rtk; settings com hooks do usuário no mesmo evento; settings com versão antiga do tokred; JSON inválido; chaves desconhecidas. Para cada uma, confirmar que configure → configure → unconfigure devolve um arquivo semanticamente igual ao original.

---

## 6. Subagentes

| Arquivo | Modelo | Uso |
|---|---|---|
| `tokred-architect` | opus | Arquitetura, decisões de design, trade-offs |
| `tokred-planner` | opus | Planejamento e quebra de tarefas grandes |
| `tokred-dev` | sonnet | Implementação, refatoração, correção de bugs |
| `tokred-reviewer` | sonnet | Revisão de código e testes |
| `tokred-explorer` | haiku | Busca no código, localizar arquivos e símbolos (somente leitura) |
| `tokred-summarizer` | haiku | Resumir docs, logs e saídas longas |

Frontmatter: `name`, `description` (quando delegar), `model`, `tools` (restringir ao necessário; explorer e summarizer só leitura).

**Regras comuns no corpo de todos os agentes** (reduzem tokens de saída e o retorno para o contexto principal):
- Não recapitular a tarefa nem descrever o processo.
- Edições pontuais em vez de reescrever arquivos inteiros.
- **Formato de retorno fixo e curto:**
  ```
  RESULTADO: <1-3 linhas>
  ARQUIVOS: <lista de paths alterados ou relevantes>
  PENDÊNCIAS: <lista ou "nenhuma">
  ```

---

## 7. Hooks

Todos os hooks: leem o stdin, respeitam o modo configurado (`off` | `warn` | `enforce`), gravam o evento em `hook_events` e seguem a regra de fail-open.

### 7.1 `route` (UserPromptSubmit)

- Classificador **heurístico em Rust**, sem LLM. Usa palavras-chave em pt-BR e en (arquitetura, design, planejar, plano → architect/planner; implementar, corrigir, bug, refatorar → dev; revisar → reviewer; onde fica, encontrar, buscar → explorer; resumir → summarizer), o tamanho do prompt e menções a arquivos.
- Saída: `additionalContext` de **uma linha**, emitida só se a classe mudou em relação ao prompt anterior da mesma sessão. Exemplo: `tokred: implementation task → delegate to the tokred-dev subagent if it involves more than one edit.`
- **Não delegar tarefa pequena:** prompts curtos e pontuais não recebem sugestão.
- Grava a classificação em `classifications`.

### 7.2 `enforce` (PreToolUse, matcher na ferramenta de subagentes: confirmar `Task`/`Agent` na doc)

- Compara o subagente escolhido com a última classificação da sessão.
- `warn`: permite e devolve um motivo curto. `enforce`: nega com um motivo que indica o agente correto.
- Agentes que não são do tokred: sempre permitir.

### 7.3 `guard-read` (PreToolUse, matcher `Read`)

1. **Bloqueio por padrão** (configurável via globs): lockfiles (`package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `poetry.lock`), `*.min.js`, `*.map`, `dist/**`, `build/**`, `node_modules/**`, `target/**`, `.git/**`. Nega com motivo curto.
2. **Arquivo grande:** acima de `max_read_bytes` (padrão 200 KB) e sem `offset`/`limit`, nega e pede leitura por faixa.
3. **Releitura:** calcula o blake3 do arquivo. Se `(session_id, path, hash)` já existe em `file_reads` com o mesmo `offset`/`limit`, nega dizendo que o conteúdo não mudou e já está no contexto. **Exceção:** se houve compactação ou `/clear` depois da leitura (detectado via evento da sessão), permitir.
4. Grava a leitura e os bytes evitados (para as métricas).

### 7.4 `guard-search` (PreToolUse, matcher `Grep|Glob`)

- Em repositório grande (mais de `large_repo_files` arquivos, contados e cacheados por raiz), negar busca sem `path` ou sem padrão restritivo. O modo padrão é `warn`.

### 7.5 `session-check` (Stop)

- Lê `transcript_path` do stdin e pega o `usage` da última resposta para estimar o tamanho do contexto.
- Acima de `context_warn_tokens` (padrão 120k), emite **mensagem para o usuário** (não para o modelo) sugerindo `/compact` ou `/clear` ao fechar a tarefa. Nunca forçar continuação.

### 7.6 `session-start` (SessionStart)

- Registra a sessão. Em `source=clear|compact`, marca um corte para que o `guard-read` volte a permitir leituras.

### 7.7 Coexistência com o rtk

- O rtk atua no PreToolUse do `Bash`; o tokred não registra matcher em `Bash`.
- O `doctor` detecta os hooks do rtk e confirma que não há sobreposição.

---

## 8. Store (SQLite)

Configuração: `journal_mode=WAL`, `busy_timeout=2000`, migrations versionadas (`user_version`).

```sql
sessions(id TEXT PK, project TEXT, started_at INT, last_seen INT, last_reset_at INT)
classifications(session_id TEXT, ts INT, class TEXT, prompt_len INT)
file_reads(session_id TEXT, path TEXT, hash TEXT, offset INT, lim INT, bytes INT, ts INT,
           PRIMARY KEY(session_id, path, hash, offset, lim))
hook_events(ts INT, session_id TEXT, hook TEXT, decision TEXT, reason TEXT, bytes_saved INT,
            duration_us INT, outcome TEXT)   -- outcome: ok | error | panic | timeout_guard
cache_entries(key TEXT PK, kind TEXT, value BLOB, tier INT, hits INT,
              created_at INT, last_hit INT, expires_at INT NULL)
installed_files(path TEXT PK, hash TEXT, version TEXT, installed_at INT)
pricing(model TEXT PK, input REAL, output REAL, cache_write REAL, cache_read REAL,
        source TEXT, updated_at INT)            -- USD por milhão de tokens
usage(message_key TEXT PK, session_id TEXT, project TEXT, ts INT, model TEXT,
      input INT, output INT, cache_write INT, cache_read INT, is_subagent INT)
ingest_state(file TEXT PK, offset INT, mtime INT)
```

### Política de cache em camadas (`cache_entries`)

| Camada | Entra quando | TTL |
|---|---|---|
| L1 | Primeira gravação | `l1_ttl` (padrão 4h) |
| L2 | `hits >= l2_hits` (padrão 3) | `l2_ttl` (padrão 24h), renovado a cada acerto |
| L3 | `hits >= l3_hits` (padrão 10) | Persistente (`expires_at = NULL`) |

- A chave sempre inclui o hash do conteúdo de origem, então a invalidação é automática quando o arquivo muda.
- A promoção acontece na leitura (incrementa `hits`, reavalia a camada).
- `cache prune` remove os expirados. Também roda de forma oportunista em `stats` e `update`, nunca dentro de hook.
- No MVP, o cache guarda: contagem de arquivos por raiz, classificações e o resultado da auditoria. Resumos de artefatos ficam para a fase 6.

---

## 9. Preços

- Fonte remota: o JSON público de preços do LiteLLM (`model_prices_and_context_window.json` no repositório BerriAI/litellm). Filtrar os modelos Claude e normalizar para USD por milhão de tokens.
- **Fallback embutido** no binário (`include_str!`), gerado no build, para quando não há rede.
- O sync roda em `configure`, `update`, `pricing sync` e na sincronização diária (seção 14), e registra `source` e `updated_at`.
- Modelo desconhecido no transcript: mapear por família (opus/sonnet/haiku) para o preço mais recente da família e sinalizar no `stats`.

---

## 10. Métricas

### Ingestão

- Ler `<claude>/projects/**/*.jsonl` de forma incremental (`ingest_state` guarda o offset e o mtime de cada arquivo).
- Extrair das mensagens do assistente: modelo, `input_tokens`, `output_tokens`, `cache_creation_input_tokens` e `cache_read_input_tokens`. **Confirmar os nomes dos campos num transcript real antes de implementar.**
- Deduplicar por id da mensagem + id da requisição, porque o streaming pode gerar linhas repetidas.
- Marcar `is_subagent` quando a linha vier de um subagente.

### Cálculos no `stats`

- **Custo real:** soma por modelo × preço.
- **Contrafactual "tudo no opus":** os mesmos tokens com preço do opus. A diferença é a **economia por roteamento**.
- **Economia das guardas:** `bytes_saved / 4` como estimativa de tokens evitados (sempre rotular como estimativa) × preço de input do modelo da sessão.
- **Taxa de cache de prompt:** `cache_read / (cache_read + cache_write + input)`.
- **Projeção:** média diária dos últimos 7 e 30 dias × 30.
- **Antes e depois:** se existirem dados anteriores ao primeiro `configure` (o transcript é mais antigo), comparar o custo por sessão antes e depois.

### Latência adicionada pelo tokred (`tokred stats latency`)

Objetivo: mostrar quanto tempo o tokred atrasou o Claude Code, já que cada hook roda de forma síncrona antes do Claude continuar.

**Medição dentro do binário**
- Capturar `Instant::now()` na primeira linha do `main` e gravar `duration_us` no fim de cada execução de hook, inclusive em erro ou panic (via guard com `Drop`).
- Gravar `outcome` (`ok`, `error`, `panic`, `timeout_guard`).
- A escrita no banco é a última etapa, depois de emitir a resposta no stdout e fechar o stdout, para não somar a própria gravação ao atraso percebido. Se a gravação falhar, ignorar (fail-open).

**Custo de inicialização do processo**
- O tempo de spawn do processo pelo Claude Code não é visível de dentro do binário. O `tokred doctor --bench` mede isso executando cada hook N vezes (padrão 50) com fixtures de stdin e cronometrando de fora (spawn → exit).
- O resultado do bench (média de overhead de spawn por SO) fica salvo no banco e é somado às durações internas no `stats latency`, rotulado como estimativa.

**O que o `stats latency` mostra**
- Por hook: número de execuções, p50, p95, p99 e máximo em ms.
- Total de ms adicionados por sessão e por dia, e média por prompt do usuário.
- Contagem de `error`, `panic` e `timeout_guard`.
- Alerta quando o p95 de qualquer hook passar do orçamento de 50 ms.
- Comparativo opcional: ms adicionados × tokens e custo economizados no mesmo período, para mostrar se a troca vale a pena.

### Relatório HTML (`tokred metrics`)

**Arquivo**
- Caminho fixo: `<dados>/reports/metrics.html`. Cada execução **sobrescreve o mesmo arquivo** (grava em temporário e faz rename atômico). Nunca gera arquivos com data no nome.
- Abre no navegador padrão com a crate `opener`, a menos que se passe `--no-open`. Imprime o caminho no terminal.

**Construção**
- HTML **autocontido e offline**: CSS e JS inline, sem CDN, sem fontes externas, sem requisições de rede.
- O template fica embutido no binário (`include_str!`). Os dados entram como um bloco JSON (`<script type="application/json" id="data">`) gerado a partir do banco.
- Gráficos em **SVG gerado em Rust** (barras, linhas e rosca), sem biblioteca de gráficos em JS. JS inline só para tooltips e para alternar o período.
- Tema claro e escuro via `prefers-color-scheme`, layout responsivo, fonte do sistema.
- Rodapé com data de geração, versão do tokred, período coberto e data da última sincronização de preços.

**Seções**
1. **Cartões de resumo:** custo real, economia total (roteamento + guardas), tokens evitados (estimativa), projeção mensal e latência total adicionada.
2. **Custo por dia:** linha do custo real × contrafactual "tudo no opus".
3. **Distribuição por modelo:** rosca com tokens e custo por opus, sonnet e haiku.
4. **Roteamento:** classificações por tipo de tarefa, delegações corretas × corrigidas pelo `enforce`.
5. **Guardas:** bloqueios por motivo (releitura, arquivo gerado, arquivo grande, busca ampla) e bytes/tokens evitados.
6. **Cache de prompt:** taxa de acerto por dia.
7. **Latência:** p50/p95/máximo por hook, com a linha do orçamento de 50 ms, e o comparativo ms adicionados × economia.
8. **Auditoria:** os itens mais pesados do contexto fixo e os MCPs sem uso.

Seções sem dados (por exemplo, antes de a fase correspondente existir) aparecem com um aviso de "sem dados ainda" em vez de sumir.

---

## 11. Auditoria (`tokred audit`)

Estimativa de tokens = caracteres / 4 (rotulada como estimativa).

- CLAUDE.md do usuário e do projeto (incluindo arquivos importados com `@`).
- Skills e agentes instalados (frontmatter e corpo).
- **MCPs:** listar os servidores configurados (settings e `~/.claude.json` do projeto; confirmar a localização na doc) × uso real nos transcripts (`tool_use` com nome `mcp__<servidor>__*`). Apontar os servidores sem uso nos últimos N dias. O peso exato dos schemas não é calculável localmente; mostrar só o uso.
- Saída: tabela ordenada por peso, com sugestão de ação curta.

---

## 12. Fases

| Fase | Entrega | Pronto quando |
|---|---|---|
| F0 | Bootstrap do repositório (seção 15), workspace com os crates da seção 3, paths, config, logging, store com migrations, `configure`/`unconfigure`/`doctor` com detecção da pasta, merge não destrutivo e agentes; README e docs nos dois idiomas (seção 16) | CI verde nos 3 SOs (fmt, clippy, testes unitários, de integração e ponta a ponta, cargo-deny); testes do merge (5.4) passando |
| F1 | Pipeline de release (`dist`, tap do brew, bucket do scoop, publicação no winget), `update` por método de instalação, pricing (sync + fallback + sync diário), verificação diária de versão com aviso, `stats` com ingestão incremental e a primeira versão do `tokred metrics` (seções 1, 2, 3 e 6) | `stats` bate com o somatório manual de um transcript de teste; o HTML abre offline e é sobrescrito a cada execução. As demais seções do HTML entram junto com as fases que geram os dados |
| F2 | `audit` | Lista o peso do contexto e os MCPs sem uso |
| F3 | `guard-read`, `guard-search`, `session-start`, medição de latência, `stats latency` e `doctor --bench` | Fixtures de stdin cobrindo permitir, negar e o corte após compactação; `doctor --bench` mostrando p95 < 50 ms |
| F4 | `route` + `enforce` | Classificador com tabela de casos de teste pt-BR/en |
| F5 | `session-check` | Aviso dispara acima do limite e nunca força continuação |
| F6 | Cache de artefatos (resumos por hash) | Só depois de medir F1 a F5 |
| F7 | Avaliar proxy/cache de respostas | Decisão registrada em DECISIONS.md com base nos dados |

---

## 13. Config (`tokred.toml`)

```toml
[modes]
route = "warn"          # off | warn | enforce
enforce = "warn"
guard_read = "enforce"
guard_search = "warn"
session_check = "warn"

[guard_read]
max_read_bytes = 204800
deny_globs = ["**/package-lock.json", "**/pnpm-lock.yaml", "**/yarn.lock",
              "**/Cargo.lock", "**/poetry.lock", "**/*.min.js", "**/*.map",
              "**/dist/**", "**/build/**", "**/node_modules/**", "**/target/**", "**/.git/**"]

[guard_search]
large_repo_files = 5000

[session_check]
context_warn_tokens = 120000

[update]
mode = "notify"         # off | notify | auto

[pricing]
auto_sync = true

[cache]
l1_ttl = "4h"
l2_ttl = "24h"
l2_hits = 3
l3_hits = 10
```

A config do usuário também passa por merge no update: as chaves novas entram com o valor padrão e os valores existentes são preservados.

---

## 14. Distribuição

Instalar e configurar são **dois passos separados**:

```sh
brew install sostenes198/tap/tokred   # 1. instala a CLI (não toca no Claude Code)
tokred configure                  # 2. integra com o Claude Code
```

### Canais de instalação

| SO | Comando | Como é publicado |
|---|---|---|
| macOS e Linux | `brew install sostenes198/tap/tokred` | Fórmula no tap `sostenes198/homebrew-tap`, gerada pelo `dist` a cada release estável |
| Windows | `winget install tokred` | Manifest enviado ao `winget-pkgs` a cada release por uma action de publicação no CI (o `dist` não gera winget nativamente; confirmar na doc atual) |
| Windows | `scoop bucket add sostenes198 https://github.com/sostenes198/scoop-bucket` + `scoop install sostenes198/tokred` | Manifest no bucket `sostenes198/scoop-bucket`, atualizado pelo CI |
| Qualquer | Script shell (`curl … \| sh`) ou PowerShell (`irm … \| iex`) | Gerados pelo `dist` |
| Qualquer | `cargo install tokred` | crates.io |

- Build com o `dist` (antigo cargo-dist) para `x86_64`/`aarch64` em Linux (musl), macOS e Windows, com checksums em cada release.
- **Nenhum canal mexe na pasta do Claude Code durante a instalação.** A fórmula do brew mostra nos caveats: `Run "tokred configure" to set up the Claude Code integration.` Os scripts imprimem a mesma mensagem no final.

### Atualização conforme o método de instalação

O tokred detecta como foi instalado pelo path do próprio executável:

| Método | Detecção (path contém) | O que `tokred update` faz |
|---|---|---|
| Homebrew | `Cellar` ou `linuxbrew` | Mostra `brew upgrade tokred` (não se atualiza sozinho) |
| winget | `WinGet\Packages` ou `WinGet\Links` | Mostra `winget upgrade tokred` |
| scoop | `scoop\apps` ou `scoop\shims` | Mostra `scoop update tokred` |
| cargo | `.cargo/bin` | Mostra `cargo install tokred --force` |
| Script | qualquer outro | Faz o self-update (download, checksum, troca atômica) |

- Depois que o binário muda de versão, a primeira execução de um **comando da CLI** compara a versão atual com a gravada no banco e, se mudou, reaplica a configuração de forma não interativa (hooks e agentes). Hooks não fazem isso; até lá continuam funcionando com a configuração anterior, e o `doctor` mostra que há reconfiguração pendente.
- CI (GitHub Actions) com matriz dos 3 SOs rodando os testes, incluindo o merge com paths do Windows.

### Verificação de nova versão

- **Uma vez por dia**, na primeira execução de um comando da CLI (`stats`, `metrics`, `audit`, `doctor`, `cache`, `pricing`), consultar a última release publicada (API de releases do GitHub).
- **Nunca dentro de hook.** O subcomando `tokred hook` não faz nenhuma verificação nem acesso à rede.
- O resultado e o horário da última verificação ficam no banco. Antes de 24h, não consulta de novo, só reaproveita o resultado salvo.
- Timeout de 2 s. Sem rede ou com erro: ignora em silêncio e tenta no dia seguinte.
- Se houver versão mais nova, imprimir **no stderr**, depois da saída do comando, uma linha com o comando certo para o método de instalação, por exemplo `new version 0.4.0 available (current 0.3.2) → run: brew upgrade tokred`. Por ir no stderr, não quebra o `--json`.
- Self-update (só para instalação via script): baixa o binário da release, **verifica o checksum**, substitui de forma atômica e reaplica a configuração. No Windows, renomeia o `.exe` antigo para `.old` antes de copiar o novo e apaga o `.old` na próxima execução.
- Config: `[update] mode = "notify"` (padrão) | `"off"` | `"auto"`. O `auto` só vale para instalação via script; em gerenciadores de pacotes ele se comporta como `notify`. Mesmo no `auto`, só roda a partir de comandos da CLI, nunca de hook. A variável de ambiente `TOKRED_NO_UPDATE_CHECK=1` desliga a verificação (útil em CI).

### Sincronização diária de preços

- Mesma regra da verificação de versão: no máximo uma vez por dia, só a partir de comandos da CLI, com timeout curto e falha silenciosa.
- É automática por padrão, porque é só dado e não código executável. Pode ser desligada com `[pricing] auto_sync = false`.

---

## 15. Bootstrap do repositório (criar do zero)

O repositório https://github.com/sostenes198/tokred começa vazio. Primeiro passo da F0:

1. `git init`, branch padrão `main`, remote `origin` apontando para `git@github.com:sostenes198/tokred.git`.
2. Criar o workspace (`Cargo.toml` raiz) com todos os crates da seção 3.1, cada um com `Cargo.toml` herdando `[workspace.package]` (version, edition, rust-version, license, repository, authors) e `lints.workspace = true`. Crates de caso de uso começam com o `lib.rs` contendo a doc de módulo e um teste trivial, para o CI ficar verde desde o primeiro commit.
3. **`.gitignore`** com o padrão do Rust e afins:
   ```gitignore
   # Rust
   /target/
   **/*.rs.bk
   *.pdb

   # Coverage / profiling
   *.profraw
   *.profdata
   lcov.info
   /coverage/

   # insta (snapshots pendentes de revisão)
   *.snap.new
   *.pending-snap

   # Local data / databases created by tests or manual runs
   *.db
   *.db-shm
   *.db-wal
   /reports/

   # Editors / IDEs
   .idea/
   .vscode/*
   !.vscode/extensions.json
   *.swp
   *.swo
   *~

   # OS
   .DS_Store
   Thumbs.db
   desktop.ini

   # Env
   .env
   .env.*
   ```
   O `Cargo.lock` **é versionado** (o projeto tem binário).
4. **`.gitattributes`:** `* text=auto eol=lf`, `*.ps1 text eol=crlf`, binários marcados como `binary`.
5. **`.editorconfig`:** UTF-8, LF, 4 espaços em `.rs`, 2 em `.toml`, `.yml`, `.json` e `.md`, newline no fim.
6. **`rust-toolchain.toml`:** canal `stable`, componentes `rustfmt` e `clippy`.
7. **`rustfmt.toml`** e **`clippy.toml`** mínimos (ex.: `max_width = 100`; `msrv` igual ao do workspace).
8. **`deny.toml`:** licenças permitidas (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib), advisories como erro, só crates.io como fonte.
9. **Licença:** padrão sugerido do ecossistema Rust, `MIT OR Apache-2.0` (dois arquivos `LICENSE-*`). **Confirmar com o autor antes de criar.**
10. **CI (`.github/workflows/ci.yml`):** em push e PR para `main`; jobs de `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace` em matriz Linux/macOS/Windows, verificação do MSRV, `cargo deny check`, `cargo doc --no-deps` sem warnings e cobertura com `cargo llvm-cov` (relatório como artefato).
11. **`dependabot.yml`** semanal para `cargo` e `github-actions`.
12. Templates de issue (bug, feature) e de PR, em inglês, com link para o `CONTRIBUTING`.
13. O `dist` (`dist init`) é configurado na F1, junto com o pipeline de release.
14. Primeiro commit: `chore: bootstrap workspace`.

---

## 16. Documentação

Toda a documentação é bilíngue (seção 0): versão em inglês no nome base e versão em português com sufixo `.pt-BR.md`. Cada arquivo começa com o seletor de idioma:

```markdown
**English** | [Português](README.pt-BR.md)
```

### 16.1 `README.md` (porta de entrada)

Curto e objetivo; detalhes ficam nos docs e o README aponta para eles. Seções, nesta ordem:

1. Nome, descrição curta e badges (CI, release, licença, crates.io).
2. **O que é** e por que existe (2 ou 3 parágrafos, mencionando o rtk).
3. **Funcionalidades** em lista curta, cada item com link para o doc correspondente.
4. **Instalação** por SO (brew, winget, scoop, script, cargo) → link para `docs/installation.md`.
5. **Início rápido:** `tokred configure`, `tokred doctor`, `tokred metrics`.
6. **Comandos** em tabela resumida → link para `docs/commands.md`.
7. **Como funciona** em um parágrafo → link para `docs/architecture.md` e `docs/hooks.md`.
8. **Configuração** → link para `docs/configuration.md`.
9. **Privacidade:** tudo local, as únicas chamadas de rede e como desligá-las.
10. **Desinstalação:** `tokred unconfigure` + comando do gerenciador de pacotes.
11. **Contribuindo** → `CONTRIBUTING.md`; **Licença**.

### 16.2 `docs/`

| Arquivo (+ `.pt-BR.md`) | Conteúdo |
|---|---|
| `docs/installation.md` | Todos os canais de instalação, atualização por método, verificação de versão, desinstalação |
| `docs/commands.md` | Referência de cada comando, flags, exemplos e saída `--json` |
| `docs/configuration.md` | Cada chave do `tokred.toml`, valores padrão e variáveis de ambiente (`TOKRED_DATA_DIR`, `TOKRED_NO_UPDATE_CHECK`, `CLAUDE_CONFIG_DIR`) |
| `docs/architecture.md` | Camadas, crates, regras de dependência e fluxo de um hook, com diagrama |
| `docs/hooks.md` | Cada hook: evento, o que decide, modos `off`/`warn`/`enforce`, como coexiste com o rtk |
| `docs/agents.md` | Os subagentes, o modelo de cada um e o formato de retorno |
| `docs/metrics.md` | Como cada número é calculado, o que é estimativa, latência e o relatório HTML |
| `docs/cache.md` | Store SQLite e a política L1/L2/L3 |
| `docs/development.md` | Requisitos, build, rodar testes (unitários, integração, ponta a ponta, snapshots), cobertura, como testar hooks localmente com fixtures, como gerar uma release |
| `docs/decisions.md` | Registro de decisões (data, contexto, decisão, consequências) |
| `docs/spec.md` | Esta especificação |

Na raiz: `CONTRIBUTING.md` (fluxo de branch e PR, Conventional Commits, checklist de testes e de docs bilíngues) e `CHANGELOG.md` (formato Keep a Changelog, atualizado a cada release).

Regras:
- Exemplos de comandos nos docs devem funcionar como estão escritos; os da seção de início rápido são cobertos por um teste ponta a ponta.
- Ao mudar comportamento, o PR atualiza o doc correspondente nas duas línguas.
