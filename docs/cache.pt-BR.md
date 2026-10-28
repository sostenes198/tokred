[English](cache.md) | **Português**

# Store e cache

O tokred guarda seu estado em um único arquivo SQLite, `<dados>/tokred.db`, aberto com `journal_mode=WAL` e `busy_timeout=2000`. O schema é versionado com `PRAGMA user_version`; cada migration roda na própria transação, e um banco criado por um tokred mais novo é recusado em vez de rebaixado.

## Tabelas

| Tabela | Guarda | Escrita por |
|---|---|---|
| `sessions` | Id da sessão, diretório de trabalho, primeira/última vez vista, último reset (`clear`/`compact`) | `session-start` |
| `classifications` | Classe acionável do prompt por sessão (nunca `other`) | `route` |
| `file_reads` | Leituras `(escopo, path, hash, offset, limit)` e bytes estimados; `offset`/`limit` ausente é gravado como `-1`; o escopo é o id da sessão, mais `/<agent_id>` dentro de um subagente | `guard-read` |
| `hook_events` | Cada execução de hook: decisão, motivo, bytes evitados, duração em µs, resultado | todos os hooks |
| `cache_entries` | Cache em camadas (abaixo) | `guard-search`, `session-check` |
| `installed_files` | Arquivos instalados pelo tokred, com hash blake3 e versão | `configure` |
| `pricing` | USD por milhão de tokens, por modelo | sync de preços |
| `usage` | Uso de tokens por mensagem, vindo dos transcripts | ingestão |
| `ingest_state` | Offsets da ingestão incremental | ingestão |
| `meta` | Pequeno estado chave/valor (checagem de versão, sync de preços, bench) | CLI |

### `hook_events`

Uma linha por execução de hook (nada é registrado no modo `off`, para nomes de hook desconhecidos ou quando não há banco). `decision` é `allow`, `warn`, `deny`, `none` ou, para o `route`, a classe da tarefa. `reason` é um entre `reread`, `generated_file`, `large_file`, `broad_search`, `agent_matches`, `agent_mismatch`, `suggested`, `unchanged`, `context_large`, `already_warned`, `below_limit`, o `source` de um `session-start`, ou o `outcome` quando a execução falhou. `outcome` é `ok`, `error`, `panic` ou `timeout_guard`. `duration_us` é medido desde a primeira linha do `main` até logo antes de a linha ser gravada, que é o último passo da execução.

## Cache em camadas (`cache_entries`)

| Camada | Entra quando | TTL |
|---|---|---|
| L1 | Primeira gravação | `l1_ttl` (padrão 4 h), contado desde a criação |
| L2 | `hits >= l2_hits` (padrão 3) | `l2_ttl` (padrão 24 h), renovado a cada acerto |
| L3 | `hits >= l3_hits` (padrão 10) | Persistente (`expires_at = NULL`) |

- A chave sempre inclui o hash do conteúdo de origem, então uma origem alterada invalida as próprias entradas automaticamente: `<tipo>:<hash>:<assunto>`. Para contagens de arquivos o "conteúdo" é a data de modificação da raiz do repositório (e o limite), o sinal mais barato de que o repositório mudou.
- A promoção acontece na leitura (`cache_get`): incrementa `hits`, reavalia a camada e renova a expiração das entradas L2. `cache_peek` lê sem contar um acerto. Uma entrada expirada é um miss.
- Um `l1_ttl` / `l2_ttl` ilegível no `tokred.toml` (qualquer coisa que não seja um número seguido de `s`, `m`, `h` ou `d`) volta ao padrão em vez de quebrar um hook.
- A política é feita de funções puras no `tokred-core` (`next_tier`, `expires_at`, `is_expired`, `record_hit`); o store só carrega e salva linhas.
- `tokred cache stats` mostra entradas, entradas expiradas, acertos e bytes por camada; `tokred cache prune` remove as expiradas. A limpeza também roda de forma oportunista em `stats` e `update`, **nunca dentro de um hook**.
- O que o cache guarda hoje: as contagens de arquivos por raiz do `guard-search` (tipo `repo_files`), a classe de cada prompt visto pelo `route`, com chave pelo blake3 do prompt e uma versão das regras (tipo `classification`), e os marcadores de "já avisou" do `session-check` (tipo `session_warn`). O histórico de classes por sessão fica na tabela `classifications`. O resultado da auditoria chega com a fase de auditoria e os resumos de artefatos na fase F6.
