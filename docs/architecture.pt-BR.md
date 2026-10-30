[English](architecture.md) | **Português**

# Arquitetura

Como o tokred é construído, para quem o altera: os crates, as regras que seguem, como um hook e uma requisição do proxy fluem e o que é guardado localmente. Para compilar e testar, veja [development](development.pt-BR.md); para saber por que algumas escolhas foram feitas, veja [decisions](decisions.pt-BR.md).

O desenho é uma arquitetura limpa para Rust: regras de negócio puras no centro, I/O nas bordas, dependências apontando para dentro e um único ponto de composição (o binário).

```text
tokred (crates/tokred-cli) ──► todos os outros crates
                                      │
  crates de casos de uso ─┐           ▼
  crates adaptadores     ─┴────► tokred-core   (tipos de domínio, modelo de config, portas; sem I/O)
```

Os crates de casos de uso e de adaptadores dependem do `tokred-core` e nunca uns dos outros (só os seus testes podem usar o `tokred-store`). Só o binário conhece todos.

## Crates

| Crate | Papel |
|---|---|
| `tokred-core` | Domínio: tipos, erros, o modelo do `tokred.toml`, regras de caminhos, o modelo de eventos de hook, a política do cache em camadas e as portas (traits como `Clock`, `FileSystem` e os repositórios). **Sem I/O** |
| `tokred-install` | `configure`, `unconfigure`, `doctor`: merge puro de settings, templates de agentes (`templates/`, embutidos no binário) e regras de instalação; o módulo `fs` é o adaptador de `std::fs` |
| `tokred-hooks` | Os sete handlers de hook, a guarda de tempo e as cargas do benchmark. Fail-open |
| `tokred-metrics` | Puro: ingestão dos transcripts, o merge do uso dos transcripts com o do proxy, custo, economia, projeção, latência e os resumos de roteamento e de guardas |
| `tokred-pricing` | Leitura da lista de preços, snapshot embutido de reserva, casos de uso de sincronização e o adaptador do LiteLLM |
| `tokred-update` | Detecção do método de instalação, verificação de versão, autoatualização com checagem de checksum, adaptador do GitHub |
| `tokred-audit` | Auditoria do contexto fixo: `CLAUDE.md` com imports, regras, skills, agentes, uso de MCP |
| `tokred-report` | O relatório HTML offline: modelo, gráficos SVG, template, escrita atômica |
| `tokred-store` | Adaptador: SQLite (`rusqlite`), migrações versionadas, implementações dos repositórios |
| `tokred-proxy` | Adaptador: o proxy local (`hyper`, `tokio`, `rustls`). Escuta em loopback, encaminha com streaming, mede uso e latência, executa o pipeline de hooks definido no `tokred-core` e serve o [dashboard](dashboard.pt-BR.md). É o único crate que depende de `hyper` e `tokio`. Lê o histórico pela porta `DashboardSource`, que o binário implementa sobre o `tokred-store` e o `tokred-report` |
| `tokred-routing` | Puro: classifica requisições de subagentes e auxiliares e escolhe um modelo mais barato quando é seguro. O modelo do loop principal nunca é trocado |
| `tokred-suggest` | Puro: resumos dos prompts do loop principal e sugestões de um modelo mais barato. Só aconselha |
| `tokred-compress` | Puro: compressão reversível de resultados grandes de ferramentas, a medição do cache de respostas e a lógica do servidor MCP `tokred_retrieve` |
| `tokred` (`crates/tokred-cli`) | O binário. Interpreta argumentos, monta os adaptadores reais, chama os casos de uso, formata a saída e decide o código de saída. É o único crate que usa `anyhow` e `println!`. O pacote se chama `tokred`; a pasta mantém o nome `tokred-cli` |

`tokred-routing`, `tokred-suggest` e `tokred-compress` se encaixam no proxy pelo pipeline `ProxyHook` definido no `tokred-core`; o `tokred-proxy` os executa sobre HTTP e o binário liga tudo e roda com `tokred proxy start --foreground`. Veja [proxy](proxy.pt-BR.md).

## Regras

- A lógica central são **funções puras** sobre dados (`merge_settings`, `remove_tokred`, `plan_agent`, `merge_config`, `merge_usage`...). O I/O entra por traits passadas como parâmetro, e o único lugar que constrói as implementações reais é o binário.
- Quando um crate faz I/O por conta própria (`tokred-install::fs`, os clientes HTTP de `tokred-pricing` e `tokred-update`), isso fica em um módulo próprio atrás de uma trait, para a lógica ao redor ser testada com fakes.
- O `Clock` existe para que TTLs, a verificação diária de versão e as métricas por dia sejam testáveis sem esperar.
- Dentro do binário, o I/O de terminal passa por uma pequena costura `Io` (flag interativa, entrada, saída) e a saída dos comandos é montada por funções puras, então prompts e relatórios são testados sem terminal.
- Erros são enums `thiserror` por crate; `anyhow` só no binário.
- Lints: `unsafe_code = forbid`; clippy `pedantic` como aviso; `unwrap`, `expect`, `panic`, `todo` e `dbg!` proibidos fora de testes (`Cargo.toml`, `[workspace.lints]`).

## Como um hook roda

1. O Claude Code executa `"<caminho>/tokred" hook <evento>` e escreve JSON no stdin.
2. O `main` trata `hook` antes de interpretar os argumentos, então até um evento desconhecido sai com 0. Um hook nunca bloqueia o Claude Code: o código de saída 2 significaria "bloquear".
3. O handler (`tokred-hooks`) roda dentro de `catch_unwind`, atrás de um watchdog (`TOKRED_HOOK_GUARD_MS`, padrão 2 s). Em erro, panic ou timeout ele registra em `<data>/logs/` e não imprime nada.
4. Uma decisão, se houver, é impressa como JSON no stdout (`hookSpecificOutput`, `permissionDecision`, `additionalContext`, `systemMessage`) e o processo sai com 0.
5. A linha do banco da execução (`hook_events`) é gravada por último. Um hook nunca cria a pasta de dados nem o banco: uma instalação removida continua removida.

## Como uma requisição do proxy flui

1. O proxy aceita a conexão em loopback e confere o cabeçalho `Host` (`421` se não bater).
2. `/__tokred/*` são endpoints administrativos: `health` é aberto; `status`, `shutdown` e `dashboard-token` exigem o segredo.
3. `/dashboard` e `/metrics` são respondidos localmente, atrás do login do navegador. Todo o resto é encaminhado.
4. Para `POST /v1/messages` com corpo JSON, o pipeline de hooks roda: medição de cache de respostas, roteamento, compressão e, por fim, resumos de prompts e sugestões. Um hook que falha ou dá panic descarta todas as suas alterações e os bytes originais são encaminhados.
5. A requisição segue ao upstream por `hyper` e `rustls` (pelo túnel de proxy HTTP e com autoridades certificadoras extras, quando configurados), e a resposta volta em streaming. Um tap lê `usage`, o motivo de parada e as chamadas de ferramentas conforme os bytes passam, sem armazenar em buffer.
6. Se a API rejeitar uma requisição reescrita (400, 404 ou 422), a original é enviada no lugar.
7. Quando a resposta termina, um registro (a linha da requisição, a decisão de roteamento, os resumos) é entregue a uma thread escritora que o grava no store, então o trabalho de banco nunca roda no caminho da requisição. Quando a fila da escritora está cheia, o registro é descartado e registrado no log, sem nunca atrasar a requisição. O estado ao vivo do dashboard e seu barramento de eventos são atualizados com os mesmos dados.

## O store local

O tokred guarda seu estado em um único arquivo SQLite, `<data>/tokred.db`, aberto com `journal_mode=WAL` e `busy_timeout=2000`.

O esquema é versionado com `PRAGMA user_version`. As migrações são strings SQL em `crates/tokred-store/src/migrations.rs`; cada uma roda na própria transação, e um banco criado por um tokred mais novo é recusado em vez de rebaixado. Para mudar o esquema, acrescente uma migração; nunca edite uma que já foi publicada.

| Tabela | Guarda |
|---|---|
| `sessions` | Id da sessão, diretório de trabalho, primeira e última vez vista, último reset (`clear` ou `compact`) |
| `classifications` | Classe acionável do prompt por sessão (nunca `other`) |
| `file_reads` | Leituras `(escopo, caminho, hash, offset, limite)` e bytes estimados; o escopo é o id da sessão, mais `/<agent_id>` dentro de um subagente |
| `hook_events` | Cada execução de hook: decisão, motivo, bytes economizados, duração em microssegundos, resultado (`ok`, `error`, `panic`, `timeout_guard`) |
| `cache_entries` | O cache em camadas (abaixo) |
| `installed_files` | Arquivos que o tokred instalou, com hash blake3 e versão |
| `pricing` | USD por milhão de tokens, por modelo |
| `usage`, `ingest_state` | Uso de tokens por mensagem vindo dos transcripts, e os offsets da ingestão incremental |
| `meta` | Pequeno estado chave/valor (verificação de versão, sincronização de preços, benchmark, marcador `tokred_version`, disjuntor da compressão) |
| `proxy_requests` | Uma linha por requisição que o proxy viu: modelos, tokens, latência, motivo de parada, ferramentas, duração do cache, modo de roteamento, causa de falha de cache |
| `routing_decisions`, `prompt_summaries`, `suggestions`, `shadow_savings` | Decisões de roteamento, resumos dos prompts do loop principal, sugestões e economias medidas |
| `stored_results`, `compressed_blocks` | Originais dos resultados de ferramentas comprimidos e as substituições enviadas, para que uma substituição seja idêntica a cada volta |
| `captured_turns` | O que o dashboard mostra de uma volta; as colunas de conteúdo ficam vazias, a menos que `[proxy] log_bodies` esteja ligado |

### Cache em camadas (`cache_entries`)

| Camada | Entra quando | Expira |
|---|---|---|
| L1 | Primeira escrita | `l1_ttl` (padrão 4 h) após a criação |
| L2 | `hits >= l2_hits` (padrão 3) | `l2_ttl` (padrão 24 h), renovado a cada acerto |
| L3 | `hits >= l3_hits` (padrão 10) | Nunca (`expires_at = NULL`) |

- As chaves incluem o hash do conteúdo de origem, então uma origem alterada invalida suas entradas: `<tipo>:<hash>:<assunto>`. Para contagens de arquivos, o conteúdo é a data de modificação da raiz do repositório mais o limite, o sinal mais barato de que o repositório mudou.
- A promoção acontece na leitura (`cache_get`): conta um acerto, reavalia a camada e renova a expiração da L2. `cache_peek` lê sem contar. Uma entrada expirada é um erro de cache.
- Um `l1_ttl` ou `l2_ttl` ilegível (qualquer coisa que não seja um número positivo mais `s`, `m`, `h` ou `d`) volta ao padrão em vez de quebrar um hook.
- A política são funções puras no `tokred-core` (`next_tier`, `expires_at`, `is_expired`, `record_hit`); o store só carrega e grava linhas.
- `tokred cache stats` mostra entradas, entradas expiradas, acertos e bytes por camada; `tokred cache prune` remove as expiradas; nada mais as limpa, e um hook nunca o faz.
- Ele guarda as contagens de arquivos por raiz do `guard-search` (tipo `repo_files`), a classe de cada prompt visto pelo `route` (tipo `classification`, com chave pelo hash blake3 do prompt mais uma versão de regras) e os marcadores "já avisado" do `session-check` (tipo `session_warn`).

## Pasta de dados

| Plataforma | Local padrão |
|---|---|
| Linux | `~/.local/share/tokred` |
| macOS | `~/Library/Application Support/tokred` |
| Windows | `%APPDATA%\tokred` |

`TOKRED_DATA_DIR` a substitui. Dentro dela:

| Caminho | O quê |
|---|---|
| `tokred.db` | O banco SQLite |
| `tokred.toml` | Configuração ([configuration](configuration.pt-BR.md)) |
| `logs/` | Os logs do próprio tokred, rotacionados por dia, 7 dias mantidos |
| `reports/` | Relatórios HTML gerados pelo `tokred metrics` |
| `proxy/secret` | O segredo administrativo do proxy (permissão só do dono no Unix) |
| `proxy/pid` | O pid e a porta do proxy em segundo plano |
| `proxy/base_urls` | As URLs de proxy que o tokred gravou nas configurações do Claude Code, uma por linha, para que uma mudança de porta seja acompanhada |

O binário e o wrapper `claude` ficam fora da pasta de dados, em `~/.tokred/bin` (Windows `%LOCALAPPDATA%\tokred\bin`); veja [installation](installation.pt-BR.md).
