[English](cache.md) | **Português**

# Store e cache

O tokred guarda seu estado em um único arquivo SQLite, `<dados>/tokred.db`, aberto com `journal_mode=WAL` e `busy_timeout=2000`. O schema é versionado com `PRAGMA user_version`; cada migration roda na própria transação, e um banco criado por um tokred mais novo é recusado em vez de rebaixado.

## Tabelas (schema v1)

| Tabela | Guarda | Usada a partir de |
|---|---|---|
| `sessions` | Id da sessão, projeto, primeira/última vez vista, último reset (`clear`/`compact`) | F3 |
| `classifications` | Classe do prompt por sessão | F4 |
| `file_reads` | Leituras `(sessão, path, hash, offset, limit)` e bytes evitados | F3 |
| `hook_events` | Cada execução de hook: decisão, motivo, bytes evitados, duração em µs, resultado | F3 |
| `cache_entries` | Cache em camadas | F1 |
| `installed_files` | Arquivos instalados pelo tokred, com hash blake3 e versão | **F0** |
| `pricing` | USD por milhão de tokens, por modelo | F1 |
| `usage` | Uso de tokens por mensagem, vindo dos transcripts | F1 |
| `ingest_state` | Offsets da ingestão incremental | F1 |

Todas as tabelas existem desde a primeira migration, então as fases seguintes não precisam mudar o schema só para começar a gravar.

## Cache em camadas (`cache_entries`)

| Camada | Entra quando | TTL |
|---|---|---|
| L1 | Primeira gravação | `l1_ttl` (padrão 4 h) |
| L2 | `hits >= l2_hits` (padrão 3) | `l2_ttl` (padrão 24 h), renovado a cada acerto |
| L3 | `hits >= l3_hits` (padrão 10) | Persistente (`expires_at = NULL`) |

- A chave sempre inclui o hash do conteúdo de origem, então um arquivo alterado invalida as próprias entradas automaticamente.
- A promoção acontece na leitura: incrementa `hits` e reavalia a camada.
- `tokred cache prune` remove os expirados; também roda de forma oportunista em `stats` e `update`, nunca dentro de um hook.
- No MVP o cache guarda contagem de arquivos por raiz, classificações e o resultado da auditoria. Resumos de artefatos ficam para a fase F6.
